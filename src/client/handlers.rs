//! What the client makes of what the server says.

use tokio::time::Instant;

use super::{App, CTCP_INTERVAL, render};
use crate::banner;
use crate::proto::numeric::*;
use crate::proto::{Message, Numeric, casemap};

impl App {
    /// Handles a line from the server.
    pub(super) fn on_line(&mut self, line: &str) {
        match Message::parse(line) {
            Ok(message) => self.on_message(&message),
            Err(_) => self.screen.push(render::info(line)),
        }
    }

    fn on_message(&mut self, msg: &Message) {
        if let Some(numeric) = msg.numeric() {
            return self.on_numeric(numeric, msg);
        }
        let nick = msg.source().unwrap_or_default();
        let arg = |i| msg.arg(i).unwrap_or_default();
        match msg.command.as_str() {
            "PING" => {
                self.send(Message::new("PONG").trailing(arg(0)));
            }
            "PONG" => {}
            "ERROR" => self.screen.push(render::error(&format!("ERROR: {}", arg(0)))),
            "PRIVMSG" => self.on_privmsg(nick, arg(0), arg(1)),
            "NOTICE" => self.on_notice(msg, arg(0), arg(1)),
            "JOIN" => self.on_join(nick, msg.source_userhost().unwrap_or_default(), arg(0)),
            "PART" => self.on_part(nick, arg(0), arg(1)),
            "KICK" => self.on_kick(nick, arg(0), arg(1), arg(2)),
            "QUIT" => {
                if self.session.quit(nick) {
                    self.screen.push(render::signoff(nick, arg(0)));
                }
            }
            "NICK" => self.on_nick(nick, arg(0)),
            "TOPIC" => {
                self.session.set_topic(arg(0), arg(1));
                let text = format!(" has changed the topic on channel {} to {}", arg(0), arg(1));
                self.screen.push(render::event(nick, &text));
            }
            "MODE" => self.on_mode(nick, msg),
            "INVITE" => {
                let text = format!(" invites you to channel {}", arg(1));
                self.screen.push(render::event(nick, &text));
            }
            "KILL" => {
                let text = format!("You have been killed by {nick} ({})", arg(1));
                self.screen.push(render::error(&text));
            }
            "WALLOPS" => self.screen.push(render::wallops(nick, arg(0))),
            _ => self.screen.push(render::info(&msg.to_string())),
        }
    }

    fn on_privmsg(&mut self, nick: &str, target: &str, text: &str) {
        if let Some((command, args)) = ctcp(text) {
            return self.on_ctcp(nick, target, command, args);
        }
        let mentions_us = mentions(text, &self.session.nick);
        if self.session.is_channel(target) {
            let here = self.session.is_current(target);
            self.screen.push(render::public(nick, (!here).then_some(target), text, mentions_us));
            self.session.mark_activity(target);
        } else {
            self.session.last_from = Some(nick.to_owned());
            self.session.open_query(nick, false);
            self.screen.push(render::private(nick, text));
            self.session.mark_activity(nick);
        }
    }

    /// A CTCP request: an action to show, or a question to answer.
    fn on_ctcp(&mut self, nick: &str, target: &str, command: &str, args: &str) {
        let command = command.to_ascii_uppercase();
        let channel = self.session.is_channel(target).then_some(target);
        if command == "ACTION" {
            let place = channel.unwrap_or(nick);
            let here = self.session.is_current(place);
            let mentions_us = mentions(args, &self.session.nick);
            self.screen.push(render::action(nick, channel.filter(|_| !here), args, mentions_us));
            if channel.is_none() {
                self.session.open_query(nick, false);
            }
            self.session.mark_activity(place);
            return;
        }
        let to = channel.map(|c| format!(" to {c}")).unwrap_or_default();
        self.screen.push(render::info(&format!("CTCP {command} from {nick}{to}")));
        let reply = match command.as_str() {
            "VERSION" => format!("VERSION sampo {} ({})", banner::VERSION, std::env::consts::OS),
            "PING" => format!("PING {args}"),
            "TIME" => format!("TIME {}", chrono::Local::now().format("%a %b %e %H:%M:%S %Y")),
            "CLIENTINFO" => String::from("CLIENTINFO ACTION CLIENTINFO PING TIME VERSION"),
            _ => return,
        };
        if self.last_ctcp.is_some_and(|last| last.elapsed() < CTCP_INTERVAL) {
            return;
        }
        self.last_ctcp = Some(Instant::now());
        self.send(Message::new("NOTICE").param(nick).trailing(format!("\x01{reply}\x01")));
    }

    fn on_notice(&mut self, msg: &Message, target: &str, text: &str) {
        let Some(nick) = msg.source().filter(|_| msg.source_userhost().is_some()) else {
            // From the server itself: ircII shows these as plain `***` news.
            let text = text.strip_prefix("*** ").unwrap_or(text);
            return self.screen.push(render::info(text));
        };
        if let Some((command, args)) = ctcp(text) {
            let command = command.to_ascii_uppercase();
            let text = match (command.as_str(), args.parse::<i64>()) {
                ("PING", Ok(sent)) => {
                    let ms = (chrono::Utc::now().timestamp_millis() - sent).max(0);
                    format!("CTCP PING reply from {nick}: {}.{:03} seconds", ms / 1000, ms % 1000)
                }
                _ => format!("CTCP {command} reply from {nick}: {args}"),
            };
            return self.screen.push(render::info(&text));
        }
        let channel = self.session.is_channel(target).then_some(target);
        let here = channel.is_some_and(|c| self.session.is_current(c));
        self.screen.push(render::notice(nick, channel.filter(|_| !here), text));
    }

    fn on_join(&mut self, nick: &str, userhost: &str, channel: &str) {
        if casemap::equal(nick, &self.session.nick) {
            self.session.joined(channel);
            self.session.mask = Some(format!("{nick}!{userhost}"));
        } else {
            self.session.add_member(channel, nick, "");
        }
        let text = format!(" ({userhost}) has joined channel {channel}");
        self.screen.push(render::event(nick, &text));
    }

    fn on_part(&mut self, nick: &str, channel: &str, reason: &str) {
        if casemap::equal(nick, &self.session.nick) {
            self.session.parted(channel);
        } else {
            self.session.remove_member(channel, nick);
        }
        let reason = if reason.is_empty() { String::new() } else { format!(" ({reason})") };
        self.screen.push(render::event(nick, &format!(" has left channel {channel}{reason}")));
    }

    fn on_kick(&mut self, by: &str, channel: &str, victim: &str, reason: &str) {
        if casemap::equal(victim, &self.session.nick) {
            self.session.parted(channel);
            let text = format!("You have been kicked off channel {channel} by {by} ({reason})");
            self.screen.push(render::error(&text));
        } else {
            self.session.remove_member(channel, victim);
            let text = format!(" has been kicked off channel {channel} by {by} ({reason})");
            self.screen.push(render::event(victim, &text));
        }
    }

    fn on_nick(&mut self, old: &str, new: &str) {
        let seen = self.session.rename(old, new);
        if casemap::equal(old, &self.session.nick) {
            self.session.nick = new.to_owned();
            self.screen.push(render::info(&format!("You are now known as {new}")));
        } else if seen {
            self.screen.push(render::event(old, &format!(" is now known as {new}")));
        }
    }

    fn on_mode(&mut self, by: &str, msg: &Message) {
        let target = msg.arg(0).unwrap_or_default();
        let modes = msg.arg(1).unwrap_or_default();
        let change = msg.params.get(1..).unwrap_or_default().join(" ");
        if self.session.is_channel(target) {
            self.session.apply_channel_modes(
                target,
                modes,
                msg.params.get(2..).unwrap_or_default(),
            );
            let text = format!("Mode change \"{change}\" on channel {target} by {by}");
            self.screen.push(render::info(&text));
        } else {
            if casemap::equal(target, &self.session.nick) {
                self.session.apply_user_modes(modes);
            }
            let text = format!("Mode change \"{change}\" for user {target} by {by}");
            self.screen.push(render::info(&text));
        }
    }

    fn on_numeric(&mut self, numeric: Numeric, msg: &Message) {
        // The first parameter of every numeric is our own nick.
        let args = msg.params.get(1..).unwrap_or_default();
        let arg = |i: usize| args.get(i).map_or("", String::as_str);
        let info = match numeric {
            RPL_WELCOME => {
                self.session.registered = true;
                if let Some(nick) = msg.arg(0) {
                    self.session.nick = nick.to_owned();
                }
                self.session.server = msg.prefix.clone();
                self.show(numeric, args);
                return self.rejoin();
            }
            RPL_ISUPPORT => {
                // All but the last parameter, which is the words "are supported...".
                let tokens = args.split_last().map(|(_, tokens)| tokens).unwrap_or_default();
                self.session.support.update(tokens);
                return self.show(numeric, args);
            }
            RPL_UMODEIS => {
                self.session.modes.clear();
                self.session.apply_user_modes(arg(0));
                format!("Your user mode is \"{}\"", arg(0))
            }
            RPL_NOWAWAY | RPL_UNAWAY => {
                self.session.away = numeric == RPL_NOWAWAY;
                arg(0).to_owned()
            }
            RPL_AWAY => format!("{} is away: {}", arg(0), arg(1)),
            RPL_WHOISUSER => format!("{} is {}@{} ({})", arg(0), arg(1), arg(2), arg(4)),
            RPL_WHOISCHANNELS => format!("on channels: {}", arg(1)),
            RPL_WHOISSERVER => format!("on irc via server {} ({})", arg(1), arg(2)),
            RPL_WHOISOPERATOR => format!("{} is an IRC operator", arg(0)),
            RPL_WHOISIDLE => {
                let idle = arg(1).parse().map_or_else(|_| arg(1).to_owned(), duration);
                format!("{} has been idle {idle}, signed on {}", arg(0), when(arg(2)))
            }
            RPL_WHOREPLY => {
                // `<channel> <user> <host> <server> <nick> <flags> :<hops> <real name>`
                let realname = arg(6).split_once(' ').map_or("", |(_, name)| name);
                format!(
                    "{:<12} {:<10} {:<4} {}@{} ({realname})",
                    arg(0),
                    arg(4),
                    arg(5),
                    arg(1),
                    arg(2)
                )
            }
            RPL_LISTSTART => format!("{:<20} {:>5}  Topic", "Channel", "Users"),
            RPL_LIST => format!("{:<20} {:>5}  {}", arg(0), arg(1), arg(2)),
            RPL_CHANNELMODEIS => {
                self.session.set_channel_modes(arg(0), arg(1));
                let modes = args.get(1..).unwrap_or_default().join(" ");
                format!("Mode for channel {} is \"{modes}\"", arg(0))
            }
            RPL_CREATIONTIME => format!("Channel {} was created {}", arg(0), when(arg(1))),
            RPL_NOTOPIC => format!("{}: No topic is set", arg(0)),
            RPL_TOPIC => {
                self.session.set_topic(arg(0), arg(1));
                format!("Topic for {}: {}", arg(0), arg(1))
            }
            RPL_TOPICWHOTIME => {
                format!("Topic for {} set by {} at {}", arg(0), arg(1), when(arg(2)))
            }
            RPL_INVITING => format!("Inviting {} to channel {}", arg(0), arg(1)),
            RPL_NAMREPLY => {
                self.session.add_names(arg(1), arg(2));
                format!("Users on {}: {}", arg(1), arg(2))
            }
            RPL_BANLIST => {
                format!("{} ban {} set by {} at {}", arg(0), arg(1), arg(2), when(arg(3)))
            }
            RPL_ISON => format!("Currently online: {}", arg(0)),
            RPL_MOTD => {
                // Each line comes as `- text`; show just the text, so that
                // pictures in the message of the day keep their shape.
                let text = arg(0).strip_prefix('-').unwrap_or(arg(0));
                return self.screen.push(render::motd(text.strip_prefix(' ').unwrap_or(text)));
            }
            RPL_ENDOFWHO | RPL_ENDOFWHOIS | RPL_ENDOFNAMES | RPL_ENDOFBANLIST | RPL_LISTEND => {
                return;
            }
            ERR_NICKNAMEINUSE | ERR_ERRONEUSNICKNAME if !self.session.registered => {
                self.show(numeric, args);
                return self.try_another_nick();
            }
            _ => return self.show(numeric, args),
        };
        self.screen.push(render::info(&info));
    }

    /// Shows a numeric reply the client has no special words for.
    fn show(&mut self, numeric: Numeric, args: &[String]) {
        if numeric.is_error() {
            let text = match args.split_last() {
                Some((last, [])) => last.clone(),
                Some((last, rest)) => format!("{}: {last}", rest.join(" ")),
                None => String::new(),
            };
            self.screen.push(render::error(&text));
        } else {
            self.screen.push(render::info(&args.join(" ")));
        }
    }

    /// Our nick was taken, or refused, before we even got in: try another,
    /// as clients always have.
    fn try_another_nick(&mut self) {
        self.session.nick_tries += 1;
        let tries = self.session.nick_tries as usize;
        if tries > 5 {
            return self.screen.push(render::error("Use /nick to choose a nickname"));
        }
        let base: String = self.options.nick.chars().take(8).collect();
        let next = if tries <= 3 {
            format!("{base}{}", "_".repeat(tries))
        } else {
            format!("guest{}", chrono::Utc::now().timestamp_subsec_micros() % 10_000)
        };
        self.screen.push(render::info(&format!("Trying the nickname {next} instead")));
        self.session.nick = next.clone();
        self.send(Message::new("NICK").param(next));
    }

    /// Joins the channels we were in before, or were asked to join.
    fn rejoin(&mut self) {
        let channels = std::mem::take(&mut self.session.rejoin);
        if !channels.is_empty() {
            self.send(Message::new("JOIN").param(channels.join(",")));
        }
    }
}

/// Splits a CTCP message, `\x01VERSION\x01`, into its command and the rest.
fn ctcp(text: &str) -> Option<(&str, &str)> {
    let inner = text.strip_prefix('\x01')?;
    // The closing ^A is often lost when a long line is cut short.
    let inner = inner.strip_suffix('\x01').unwrap_or(inner);
    Some(inner.split_once(' ').unwrap_or((inner, "")))
}

/// Whether `text` mentions `nick` as a word of its own.
fn mentions(text: &str, nick: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || "[]\\`_^{|}-".contains(c)))
        .any(|word| casemap::equal(word, nick))
}

/// A Unix time as a date, in local time: `Tue Oct 20 12:00:00 2026`.
fn when(timestamp: &str) -> String {
    timestamp
        .parse()
        .ok()
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map_or_else(
            || timestamp.to_owned(),
            |time| time.with_timezone(&chrono::Local).format("%a %b %e %H:%M:%S %Y").to_string(),
        )
}

/// A span of seconds in words: `3 minutes 12 seconds`.
fn duration(seconds: u64) -> String {
    let units = [(86_400, "day"), (3_600, "hour"), (60, "minute"), (1, "second")];
    let mut words = Vec::new();
    let mut rest = seconds;
    for (size, name) in units {
        let count = rest / size;
        rest %= size;
        if count > 0 && words.len() < 2 {
            words.push(format!("{count} {name}{}", if count == 1 { "" } else { "s" }));
        }
    }
    if words.is_empty() { String::from("0 seconds") } else { words.join(" ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctcp_messages_come_apart() {
        assert_eq!(ctcp("\x01VERSION\x01"), Some(("VERSION", "")));
        assert_eq!(ctcp("\x01ACTION waves hello\x01"), Some(("ACTION", "waves hello")));
        assert_eq!(ctcp("\x01ACTION cut short"), Some(("ACTION", "cut short")));
        assert_eq!(ctcp("plain text"), None);
    }

    #[test]
    fn mentions_are_whole_words() {
        assert!(mentions("alice: hello", "alice"));
        assert!(mentions("hi ALICE!", "alice"));
        assert!(mentions("ping [ilmari]", "{ilmari}"));
        assert!(!mentions("malice aforethought", "alice"));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(0), "0 seconds");
        assert_eq!(duration(1), "1 second");
        assert_eq!(duration(192), "3 minutes 12 seconds");
        assert_eq!(duration(90_061), "1 day 1 hour");
    }
}
