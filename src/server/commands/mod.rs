//! The commands the server understands, and the table that routes them.

mod channel;
mod info;
mod message;
mod mode;
mod oper;
mod user;

use tokio::time::Instant;

use super::{CHANNELLEN, ClientId, ErrorReply, NICKLEN, Server};
use crate::proto::Message;

/// What a command handler comes back with: nothing, or an error reply for
/// the client that sent the command.
type Outcome = Result<(), ErrorReply>;

/// A command, and what it takes to run it.
struct Command {
    name: &'static str,
    /// How many parameters there must be before the handler is called.
    min_params: usize,
    /// Whether the sender must have registered first.
    registered: bool,
    handler: fn(&mut Server, ClientId, &Message) -> Outcome,
}

/// Every command the server knows: `msgtab[]`, as in the original ircd's
/// parse.c.
#[rustfmt::skip]
static COMMANDS: &[Command] = &[
    Command { name: "PASS",     min_params: 1, registered: false, handler: Server::pass },
    Command { name: "NICK",     min_params: 0, registered: false, handler: Server::nick },
    Command { name: "USER",     min_params: 4, registered: false, handler: Server::user },
    Command { name: "QUIT",     min_params: 0, registered: false, handler: Server::quit },
    Command { name: "PING",     min_params: 0, registered: false, handler: Server::ping },
    Command { name: "PONG",     min_params: 0, registered: false, handler: Server::pong },
    Command { name: "PRIVMSG",  min_params: 0, registered: true,  handler: Server::privmsg },
    Command { name: "NOTICE",   min_params: 0, registered: true,  handler: Server::notice },
    Command { name: "JOIN",     min_params: 1, registered: true,  handler: Server::join },
    Command { name: "PART",     min_params: 1, registered: true,  handler: Server::part },
    Command { name: "TOPIC",    min_params: 1, registered: true,  handler: Server::topic },
    Command { name: "NAMES",    min_params: 0, registered: true,  handler: Server::names },
    Command { name: "LIST",     min_params: 0, registered: true,  handler: Server::list },
    Command { name: "INVITE",   min_params: 2, registered: true,  handler: Server::invite },
    Command { name: "KICK",     min_params: 2, registered: true,  handler: Server::kick },
    Command { name: "MODE",     min_params: 1, registered: true,  handler: Server::mode },
    Command { name: "WHO",      min_params: 0, registered: true,  handler: Server::who },
    Command { name: "WHOIS",    min_params: 0, registered: true,  handler: Server::whois },
    Command { name: "USERHOST", min_params: 1, registered: true,  handler: Server::userhost },
    Command { name: "ISON",     min_params: 1, registered: true,  handler: Server::ison },
    Command { name: "AWAY",     min_params: 0, registered: true,  handler: Server::away },
    Command { name: "MOTD",     min_params: 0, registered: true,  handler: Server::motd },
    Command { name: "LUSERS",   min_params: 0, registered: true,  handler: Server::lusers },
    Command { name: "VERSION",  min_params: 0, registered: true,  handler: Server::version },
    Command { name: "TIME",     min_params: 0, registered: true,  handler: Server::time },
    Command { name: "INFO",     min_params: 0, registered: true,  handler: Server::info },
    Command { name: "OPER",     min_params: 2, registered: true,  handler: Server::oper },
    Command { name: "KILL",     min_params: 1, registered: true,  handler: Server::kill },
    Command { name: "WALLOPS",  min_params: 1, registered: true,  handler: Server::wallops },
    Command { name: "DIE",      min_params: 0, registered: true,  handler: Server::die },
];

impl Server {
    /// Routes a client's message to its handler: `parse()` in ircd's parse.c.
    pub(super) fn dispatch(&mut self, id: ClientId, message: Message) {
        let Some(client) = self.clients.get_mut(&id) else {
            return;
        };
        client.last_heard = Instant::now();
        client.ping_sent = None;
        let registered = client.registered;

        // A numeric from a client means nothing, and gets no answer.
        if message.numeric().is_some() {
            return;
        }
        let Some(command) = COMMANDS.iter().find(|c| c.name == message.command) else {
            return self.send_error(id, ErrorReply::UnknownCommand(message.command));
        };
        let outcome = if command.registered && !registered {
            Err(ErrorReply::NotRegistered)
        } else if message.params.len() < command.min_params {
            Err(ErrorReply::NeedMoreParams(command.name))
        } else {
            (command.handler)(self, id, &message)
        };
        if let Err(error) = outcome {
            self.send_error(id, error);
        }
    }
}

/// Seconds since the epoch.
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Cuts `text` to at most `max` bytes, on a character boundary.
fn clip(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Whether a nickname is well formed (RFC 2812 §2.3.1): a letter or one of
/// ``[]\`_^{|}`` first, then those, digits, and hyphens.
fn valid_nick(nick: &str) -> bool {
    let special = |c: char| "[]\\`_^{|}".contains(c);
    let mut chars = nick.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || special(c));
    first_ok
        && nick.len() <= NICKLEN
        && chars.all(|c| c.is_ascii_alphanumeric() || special(c) || c == '-')
}

/// Whether a channel name is well formed (RFC 1459 §1.3): `#` and then
/// anything but spaces, commas and control characters, fifty bytes at most.
fn valid_channel(name: &str) -> bool {
    name.starts_with('#')
        && name.len() > 1
        && name.len() <= CHANNELLEN
        && !name.contains(|c: char| c == ' ' || c == ',' || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nicknames() {
        for good in
            ["alice", "Bob", "[ilmari]", "x-1", "_under", "a|b", "`grave", "vainamoinen1988"]
        {
            assert!(valid_nick(good), "{good} should be valid");
        }
        for bad in ["", "1abc", "-dash", "has space", "na.me", "väinö", "seventeen_letters"] {
            assert!(!valid_nick(bad), "{bad} should be invalid");
        }
    }

    #[test]
    fn channel_names() {
        assert!(valid_channel("#sampo"));
        assert!(valid_channel("#Ääkköset"));
        for bad in ["sampo", "#", "#a,b", "#a b", "#bell\x07", &format!("#{}", "x".repeat(50))] {
            assert!(!valid_channel(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn clipping_respects_characters() {
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("hello", 4), "hell");
        assert_eq!(clip("ääää", 3), "ä");
    }

    #[test]
    fn the_table_is_in_upper_case_and_has_no_duplicates() {
        for (i, command) in COMMANDS.iter().enumerate() {
            assert_eq!(command.name, command.name.to_ascii_uppercase());
            assert!(COMMANDS[i + 1..].iter().all(|c| c.name != command.name));
        }
    }
}
