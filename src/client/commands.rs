//! The commands typed at the prompt.

use super::text::Line;
use super::{App, parse_address, render};
use crate::proto::{MAX_LINE, Message};

/// What `/help` says, command by command.
const HELP: &[(&str, &str)] = &[
    ("join #channel [key]", "join a channel, or switch to one you are in"),
    ("part [#channel] [message]", "leave a channel"),
    ("msg nick|#channel text", "send a message; , is whoever wrote last, . whoever you did"),
    ("query [nick]", "talk to someone by default; on its own, stop"),
    ("me action", "act something out"),
    ("notice target text", "send a notice"),
    ("nick nickname", "change your nickname"),
    ("topic [#channel] [text]", "show the topic, or set it"),
    ("names [#channel]", "see who is in a channel"),
    ("who [mask]", "list users"),
    ("whois nick", "find out about someone"),
    ("list [#channel]", "list channels"),
    ("mode [target] modes", "change modes; +o etc. apply to the current channel"),
    ("umode modes", "change your own modes"),
    ("op|deop|voice|devoice nick...", "hand out or take away status"),
    ("kick nick [reason]", "remove someone from the current channel"),
    ("invite nick [#channel]", "invite someone in"),
    ("away [message]", "mark yourself away; on its own, back again"),
    ("ctcp nick request", "send a CTCP request, such as VERSION"),
    ("ping nick", "measure the round trip to someone"),
    ("quote line", "send a raw line to the server"),
    ("server [host[:port]]", "connect to a server, or connect again"),
    ("clear", "clear the screen"),
    ("quit [message]", "leave"),
];

/// Every command name, for Tab: the client's own, and those it passes
/// straight on to the server.
pub(super) const NAMES: &[&str] = &[
    "away", "back", "clear", "ctcp", "deop", "devoice", "die", "help", "info", "invite", "join",
    "kick", "kill", "list", "lusers", "me", "mode", "motd", "msg", "names", "nick", "notice", "op",
    "oper", "part", "ping", "query", "quit", "quote", "server", "time", "topic", "umode",
    "version", "voice", "wallops", "who", "whois",
];

impl App {
    /// Acts on a line typed at the prompt.
    pub(super) fn on_submit(&mut self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        match line.strip_prefix('/') {
            // A doubled slash sends a line that starts with a slash.
            Some(rest) if rest.starts_with('/') => self.say_here(rest),
            Some(rest) => {
                let (name, args) = split_word(rest);
                self.command(&name.to_ascii_lowercase(), args);
            }
            None => self.say_here(line),
        }
    }

    fn command(&mut self, name: &str, args: &str) {
        let (first, rest) = split_word(args);
        match name {
            "join" | "j" => self.join(first, rest),
            "part" | "leave" => self.part(args),
            "msg" | "m" => self.msg(first, rest),
            "query" | "q" => self.query(first),
            "me" => self.me(args),
            "notice" => self.notice(first, rest),
            "nick" if !first.is_empty() => {
                self.send(Message::new("NICK").param(first));
            }
            "topic" | "t" => self.topic(args),
            "names" | "n" => self.ask("NAMES", first),
            "who" => self.ask("WHO", first),
            "whois" | "wi" => self.whois(first),
            "list" => {
                self.send(Message::new("LIST").params((!first.is_empty()).then_some(first)));
            }
            "mode" => self.mode(args),
            "umode" => {
                let nick = self.session.nick.clone();
                self.send(Message::new("MODE").param(nick).params(args.split_whitespace()));
            }
            "op" | "deop" | "voice" | "devoice" => self.status(name, args),
            "kick" | "k" => self.kick(args),
            "invite" => self.invite(first, rest),
            "away" => {
                let away = Message::new("AWAY");
                self.send(if args.is_empty() { away } else { away.trailing(args) });
            }
            "back" => {
                self.send(Message::new("AWAY"));
            }
            "ctcp" => self.ctcp(first, rest),
            "ping" => {
                let now = chrono::Utc::now().timestamp_millis();
                self.ctcp(first, &format!("PING {now}"));
            }
            "quote" | "raw" => self.quote(args),
            "server" | "connect" => self.server(first, rest),
            "clear" => self.screen.clear(),
            "help" => self.help(),
            "quit" | "exit" | "bye" | "signoff" => self.quit(Some(args).filter(|a| !a.is_empty())),
            "kill" if !first.is_empty() => {
                let kill = Message::new("KILL").param(first);
                self.send(if rest.is_empty() { kill } else { kill.trailing(rest) });
            }
            "wallops" if !args.is_empty() => {
                self.send(Message::new("WALLOPS").trailing(args));
            }
            "nick" | "kill" | "wallops" => self.usage(name),
            // Anything else goes to the server as it stands, as in ircII.
            _ => self.quote(&format!("{} {args}", name.to_ascii_uppercase())),
        }
    }

    /// Says something to the current channel or conversation.
    fn say_here(&mut self, text: &str) {
        match self.session.current.clone() {
            Some(target) => self.say(&target, text),
            None => {
                self.screen.push(render::error("You have not joined a channel. Try /join #sampo"))
            }
        }
    }

    /// Sends a message, cut into pieces that fit, and shows it.
    fn say(&mut self, target: &str, text: &str) {
        let here = self.session.is_current(target);
        for piece in split_text(text, self.room(target)) {
            if !self.send(Message::new("PRIVMSG").param(target).trailing(piece)) {
                return;
            }
            let line =
                if here { render::own_public(piece) } else { render::own_private(target, piece) };
            self.screen.push(line);
        }
    }

    /// How many bytes of text fit in a PRIVMSG to `target`, once the server
    /// has put our whole `nick!user@host` in front of it.
    fn room(&self, target: &str) -> usize {
        let prefix = match &self.session.mask {
            Some(mask) => mask.len(),
            // nick!~user@host, at their longest.
            None => self.session.nick.len() + 1 + 10 + 1 + 63,
        };
        let overhead = 1 + prefix + " PRIVMSG ".len() + target.len() + " :".len() + 2;
        MAX_LINE.saturating_sub(overhead).max(64)
    }

    fn join(&mut self, channels: &str, key: &str) {
        if channels.is_empty() {
            return self.usage("join");
        }
        // `/join sampo` means #sampo.
        let names: Vec<String> =
            channels
                .split(',')
                .filter(|name| !name.is_empty())
                .map(|name| {
                    if self.session.is_channel(name) { name.to_owned() } else { format!("#{name}") }
                })
                .collect();
        // Already in it? Then this is only a change of channel.
        if let [name] = names.as_slice()
            && let Some(channel) = self.session.channel(name)
        {
            let name = channel.name.clone();
            return self.session.switch_to(&name);
        }
        let join = Message::new("JOIN").param(names.join(","));
        self.send(if key.is_empty() { join } else { join.param(key) });
    }

    fn part(&mut self, args: &str) {
        let Some((channel, reason)) = self.channel_and_rest(args) else {
            return self.not_in_a_channel();
        };
        let part = Message::new("PART").param(channel);
        self.send(if reason.is_empty() { part } else { part.trailing(reason) });
    }

    fn msg(&mut self, target: &str, text: &str) {
        let target = match target {
            "," => self.session.last_from.clone(),
            "." => self.session.last_to.clone(),
            "" => None,
            target => Some(target.to_owned()),
        };
        let Some(target) = target.filter(|_| !text.is_empty()) else {
            return self.usage("msg");
        };
        if !self.session.is_channel(&target) {
            self.session.last_to = Some(target.clone());
        }
        self.say(&target, text);
    }

    fn query(&mut self, nick: &str) {
        if !nick.is_empty() {
            self.session.open_query(nick, true);
            return self.screen.push(render::info(&format!("Talking to {nick}")));
        }
        match self.session.current.clone() {
            Some(nick) if !self.session.is_channel(&nick) => {
                self.session.close(&nick);
                self.screen.push(render::info(&format!("No longer talking to {nick}")));
            }
            _ => self.usage("query"),
        }
    }

    fn me(&mut self, action: &str) {
        let Some(target) = self.session.current.clone() else {
            return self.not_in_a_channel();
        };
        if action.is_empty() {
            return self.usage("me");
        }
        let request = format!("\x01ACTION {action}\x01");
        if self.send(Message::new("PRIVMSG").param(target).trailing(request)) {
            let nick = self.session.nick.clone();
            self.screen.push(render::action(&nick, None, action, false));
        }
    }

    fn notice(&mut self, target: &str, text: &str) {
        if target.is_empty() || text.is_empty() {
            return self.usage("notice");
        }
        if self.send(Message::new("NOTICE").param(target).trailing(text)) {
            self.screen.push(render::own_notice(target, text));
        }
    }

    fn topic(&mut self, args: &str) {
        let Some((channel, text)) = self.channel_and_rest(args) else {
            return self.not_in_a_channel();
        };
        let topic = Message::new("TOPIC").param(channel);
        self.send(if text.is_empty() { topic } else { topic.trailing(text) });
    }

    /// NAMES or WHO, of the channel named or else the current one.
    fn ask(&mut self, command: &str, target: &str) {
        let target = match target {
            "" => self.session.current_channel().map(str::to_owned),
            target => Some(target.to_owned()),
        };
        self.send(Message::new(command).params(target));
    }

    fn whois(&mut self, nick: &str) {
        let nick = match nick {
            "" => self.session.current.clone().filter(|t| !self.session.is_channel(t)),
            nick => Some(nick.to_owned()),
        };
        match nick {
            Some(nick) => {
                self.send(Message::new("WHOIS").param(nick));
            }
            None => self.usage("whois"),
        }
    }

    fn mode(&mut self, args: &str) {
        let (first, rest) = split_word(args);
        let here = self.session.current_channel().map(str::to_owned);
        let mode = match (first, here) {
            ("", Some(channel)) => Message::new("MODE").param(channel),
            ("", None) => Message::new("MODE").param(self.session.nick.clone()),
            (first, Some(channel)) if first.starts_with(['+', '-']) => {
                Message::new("MODE").param(channel).params(args.split_whitespace())
            }
            (target, _) => Message::new("MODE").param(target).params(rest.split_whitespace()),
        };
        self.send(mode);
    }

    /// `/op`, `/deop`, `/voice` and `/devoice`, four nicks at a time.
    fn status(&mut self, which: &str, nicks: &str) {
        let Some(channel) = self.session.current_channel().map(str::to_owned) else {
            return self.not_in_a_channel();
        };
        let nicks: Vec<&str> = nicks.split_whitespace().take(4).collect();
        if nicks.is_empty() {
            return self.usage("op");
        }
        let (sign, letter) = match which {
            "op" => ('+', 'o'),
            "deop" => ('-', 'o'),
            "voice" => ('+', 'v'),
            _ => ('-', 'v'),
        };
        let modes: String =
            std::iter::once(sign).chain(std::iter::repeat_n(letter, nicks.len())).collect();
        self.send(Message::new("MODE").param(channel).param(modes).params(nicks));
    }

    fn kick(&mut self, args: &str) {
        let Some((channel, rest)) = self.channel_and_rest(args) else {
            return self.not_in_a_channel();
        };
        let (nick, reason) = split_word(rest);
        if nick.is_empty() {
            return self.usage("kick");
        }
        let kick = Message::new("KICK").param(channel).param(nick);
        self.send(if reason.is_empty() { kick } else { kick.trailing(reason) });
    }

    fn invite(&mut self, nick: &str, channel: &str) {
        let (channel, _) = split_word(channel);
        let channel = match channel {
            "" => self.session.current_channel().map(str::to_owned),
            channel => Some(channel.to_owned()),
        };
        match channel.filter(|_| !nick.is_empty()) {
            Some(channel) => {
                self.send(Message::new("INVITE").param(nick).param(channel));
            }
            None => self.usage("invite"),
        }
    }

    fn ctcp(&mut self, target: &str, request: &str) {
        if target.is_empty() || request.is_empty() {
            return self.usage("ctcp");
        }
        let (command, rest) = split_word(request);
        let command = command.to_ascii_uppercase();
        let body = if rest.is_empty() { command.clone() } else { format!("{command} {rest}") };
        if self.send(Message::new("PRIVMSG").param(target).trailing(format!("\x01{body}\x01"))) {
            self.screen.push(render::info(&format!("Sent a CTCP {command} to {target}")));
        }
    }

    fn quote(&mut self, line: &str) {
        match Message::parse(line) {
            Ok(message) => {
                self.send(message);
            }
            Err(error) => self.screen.push(render::error(&format!("Cannot send that: {error}"))),
        }
    }

    fn server(&mut self, address: &str, port: &str) {
        if !address.is_empty() {
            match parse_address(address) {
                Ok((host, given)) => {
                    let port = given.or_else(|| port.parse().ok()).unwrap_or(self.options.port);
                    (self.options.host, self.options.port) = (host, port);
                }
                Err(why) => return self.screen.push(render::error(&why)),
            }
        }
        self.connect(self.options.host.clone(), self.options.port);
    }

    fn help(&mut self) {
        self.screen.push(render::info("Commands:"));
        for (usage, what) in HELP {
            let line = Line::now()
                .span(&format!("  /{usage:<30} "), render::style::bold())
                .span(what, render::style::plain());
            self.screen.push(line);
        }
        for text in [
            "Anything else after a / goes to the server as it is: /motd, /lusers, /time, /oper...",
            "Keys: Tab completes, Up and Down recall, PgUp and PgDn scroll, Ctrl-X goes to the",
            "next channel, Alt-1 to Alt-9 straight to one, Ctrl-L redraws, Ctrl-C quits.",
        ] {
            self.screen.push(render::info(text));
        }
    }

    fn usage(&mut self, command: &str) {
        let usage = HELP
            .iter()
            .find(|(usage, _)| usage.split([' ', '|']).next() == Some(command))
            .map_or(command, |(usage, _)| usage);
        self.screen.push(render::error(&format!("Usage: /{usage}")));
    }

    fn not_in_a_channel(&mut self) {
        self.screen.push(render::error("You are not in a channel. Name one, or /join one"));
    }

    /// A channel named at the front of `args`, or else the current one,
    /// and whatever follows it.
    fn channel_and_rest<'a>(&self, args: &'a str) -> Option<(String, &'a str)> {
        let (first, rest) = split_word(args);
        if self.session.is_channel(first) {
            Some((first.to_owned(), rest))
        } else {
            self.session.current_channel().map(|channel| (channel.to_owned(), args))
        }
    }
}

/// The first word, and the rest after the spaces that follow it.
fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.split_once(' ') {
        Some((word, rest)) => (word, rest.trim_start()),
        None => (text, ""),
    }
}

/// Cuts text into pieces of at most `max` bytes, at spaces where it can.
fn split_text(text: &str, max: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while rest.len() > max {
        let mut cut = max;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(space) = rest[..cut].rfind(' ').filter(|&at| at > max / 2) {
            cut = space;
        }
        pieces.push(&rest[..cut]);
        rest = rest[cut..].strip_prefix(' ').unwrap_or(&rest[cut..]);
    }
    pieces.push(rest);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_off_the_front() {
        assert_eq!(split_word("bob  hello there"), ("bob", "hello there"));
        assert_eq!(split_word("  alone"), ("alone", ""));
        assert_eq!(split_word(""), ("", ""));
    }

    #[test]
    fn long_text_is_cut_at_spaces() {
        let text = "aaaa bbbb cccc dddd";
        assert_eq!(split_text(text, 10), ["aaaa bbbb", "cccc dddd"]);
        assert_eq!(split_text("short", 10), ["short"]);
        let unbroken = "x".repeat(25);
        assert_eq!(split_text(&unbroken, 10).len(), 3);
        let pieces = split_text("ääääää", 5);
        assert!(pieces.iter().all(|p| p.len() <= 5));
        assert_eq!(pieces.concat(), "ääääää");
    }

    #[test]
    fn every_command_in_the_help_can_be_completed() {
        for (usage, _) in HELP {
            let name = usage.split([' ', '|']).next().unwrap();
            assert!(NAMES.contains(&name), "{name} is missing from NAMES");
        }
    }
}
