//! IRC messages: reading them off the wire, and writing them back.
//!
//! RFC 1459 §2.3.1 gives the grammar:
//!
//! ```text
//! <message>  ::= [':' <prefix> <SPACE> ] <command> <params> <crlf>
//! <prefix>   ::= <servername> | <nick> [ '!' <user> ] [ '@' <host> ]
//! <command>  ::= <letter> { <letter> } | <number> <number> <number>
//! <params>   ::= <SPACE> [ ':' <trailing> | <middle> <params> ]
//! ```
//!
//! Only the last parameter may contain spaces. It is marked with a colon
//! and called the *trailing* parameter.

use std::fmt;
use std::str::FromStr;

use super::{MAX_LINE, Numeric};

/// The most parameters one message may carry.
pub const MAX_PARAMS: usize = 15;

/// A single IRC message.
///
/// Build one with [`Message::new`] and the chained setters, read one with
/// [`Message::parse`], and write one with [`Message::to_line`] or `Display`.
///
/// ```
/// use sampo::proto::Message;
///
/// let msg = Message::new("PRIVMSG").param("#sampo").trailing("hello, world");
/// assert_eq!(msg.to_string(), "PRIVMSG #sampo :hello, world");
///
/// let back: Message = "PRIVMSG #sampo :hello, world".parse().unwrap();
/// assert_eq!(back, msg);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Where the message came from: a server name, or `nick!user@host`.
    pub prefix: Option<String>,
    /// The command in upper case, or a three-digit numeric reply.
    pub command: String,
    /// The parameters, the trailing one included.
    pub params: Vec<String>,
    /// Whether the last parameter is written in trailing form (`:text`)
    /// even when it has no spaces in it. Free text always is.
    trailing: bool,
}

impl Message {
    /// Starts a message with the given command and no parameters.
    pub fn new(command: impl Into<String>) -> Self {
        Message { prefix: None, command: command.into(), params: Vec::new(), trailing: false }
    }

    /// Sets the prefix.
    #[must_use]
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Appends a parameter. It must not contain spaces.
    #[must_use]
    pub fn param(mut self, param: impl Into<String>) -> Self {
        self.params.push(param.into());
        self.trailing = false;
        self
    }

    /// Appends several parameters. None of them may contain spaces.
    #[must_use]
    pub fn params<I>(mut self, params: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        let before = self.params.len();
        self.params.extend(params.into_iter().map(Into::into));
        if self.params.len() > before {
            self.trailing = false;
        }
        self
    }

    /// Appends the final, free-text parameter.
    #[must_use]
    pub fn trailing(mut self, text: impl Into<String>) -> Self {
        self.params.push(text.into());
        self.trailing = true;
        self
    }

    /// Reads a message from one line, with or without its CR-LF.
    ///
    /// The parser is as forgiving as ircd always was: runs of spaces count
    /// as one, a fifteenth parameter swallows the rest of the line, and
    /// IRCv3 message tags (never negotiated here) are stepped over.
    pub fn parse(line: &str) -> Result<Message, ParseError> {
        let mut rest = line.trim_end_matches(['\r', '\n']);

        if rest.starts_with('@') {
            rest = rest.split_once(' ').map_or("", |(_, after)| after);
        }

        rest = rest.trim_start_matches(' ');
        let mut prefix = None;
        if let Some(after) = rest.strip_prefix(':') {
            let (source, after) = after.split_once(' ').unwrap_or((after, ""));
            prefix = (!source.is_empty()).then(|| source.to_owned());
            rest = after.trim_start_matches(' ');
        }

        let (command, mut rest) = rest.split_once(' ').unwrap_or((rest, ""));
        if command.is_empty() {
            return Err(ParseError::NoCommand);
        }
        let word = command.bytes().all(|b| b.is_ascii_alphabetic());
        if !word && Numeric::parse(command).is_none() {
            return Err(ParseError::BadCommand(command.to_owned()));
        }

        let mut params = Vec::new();
        let mut trailing = false;
        loop {
            rest = rest.trim_start_matches(' ');
            if rest.is_empty() {
                break;
            }
            if let Some(text) = rest.strip_prefix(':') {
                params.push(text.to_owned());
                trailing = true;
                break;
            }
            if params.len() == MAX_PARAMS - 1 {
                params.push(rest.to_owned());
                trailing = true;
                break;
            }
            let (param, after) = rest.split_once(' ').unwrap_or((rest, ""));
            params.push(param.to_owned());
            rest = after;
        }

        Ok(Message { prefix, command: command.to_ascii_uppercase(), params, trailing })
    }

    /// The parameter at `index`, if there is one.
    pub fn arg(&self, index: usize) -> Option<&str> {
        self.params.get(index).map(String::as_str)
    }

    /// The reply code, if this is a numeric reply.
    pub fn numeric(&self) -> Option<Numeric> {
        Numeric::parse(&self.command)
    }

    /// Who sent the message: the nick from a `nick!user@host` prefix, or
    /// the whole prefix when it names a server.
    pub fn source(&self) -> Option<&str> {
        let prefix = self.prefix.as_deref()?;
        prefix.split(['!', '@']).next()
    }

    /// The `user@host` half of a `nick!user@host` prefix.
    pub fn source_userhost(&self) -> Option<&str> {
        let (_, userhost) = self.prefix.as_deref()?.split_once('!')?;
        Some(userhost)
    }

    /// The message as it goes on the wire: CR-LF terminated, and cut to
    /// 512 bytes if it would be longer, as ircd always did.
    pub fn to_line(&self) -> String {
        let mut line = self.to_string();
        let max = MAX_LINE - 2;
        if line.len() > max {
            let mut end = max;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            line.truncate(end);
        }
        line.push_str("\r\n");
        line
    }
}

impl fmt::Display for Message {
    /// Writes the message as it appears on the wire, without the CR-LF.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = &self.prefix {
            write!(f, ":{} ", Wire(prefix))?;
        }
        f.write_str(&self.command)?;
        if let Some((last, middle)) = self.params.split_last() {
            for param in middle {
                write!(f, " {}", Wire(param))?;
            }
            let colon =
                self.trailing || last.is_empty() || last.starts_with(':') || last.contains(' ');
            let colon = if colon { ":" } else { "" };
            write!(f, " {colon}{}", Wire(last))?;
        }
        Ok(())
    }
}

impl FromStr for Message {
    type Err = ParseError;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        Message::parse(line)
    }
}

/// A parameter on its way to the wire, with CR, LF and NUL blanked out:
/// whatever it holds, it must never end the line early and smuggle in
/// another.
struct Wire<'a>(&'a str);

impl fmt::Display for Wire<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rest = self.0;
        while let Some(at) = rest.find(['\r', '\n', '\0']) {
            f.write_str(&rest[..at])?;
            f.write_str(" ")?;
            rest = &rest[at + 1..];
        }
        f.write_str(rest)
    }
}

/// Why a line could not be read as a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// There was no command: the line was blank, or held only a prefix.
    NoCommand,
    /// The command was neither a word nor a three-digit number.
    BadCommand(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::NoCommand => f.write_str("message has no command"),
            ParseError::BadCommand(command) => write!(f, "{command:?} is not a command"),
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Message {
        Message::parse(line).unwrap()
    }

    #[test]
    fn reads_a_bare_command() {
        let msg = parse("QUIT");
        assert_eq!(msg.prefix, None);
        assert_eq!(msg.command, "QUIT");
        assert!(msg.params.is_empty());
    }

    #[test]
    fn reads_prefix_params_and_trailing() {
        let msg = parse(":alice!~alice@10.0.0.7 PRIVMSG #sampo :hello, world\r\n");
        assert_eq!(msg.prefix.as_deref(), Some("alice!~alice@10.0.0.7"));
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, ["#sampo", "hello, world"]);
        assert_eq!(msg.source(), Some("alice"));
        assert_eq!(msg.source_userhost(), Some("~alice@10.0.0.7"));
    }

    #[test]
    fn upper_cases_the_command() {
        assert_eq!(parse("privmsg bob :hi").command, "PRIVMSG");
    }

    #[test]
    fn forgives_extra_spaces() {
        let msg = parse("  MODE   #sampo  +o   alice  ");
        assert_eq!(msg.params, ["#sampo", "+o", "alice"]);
    }

    #[test]
    fn keeps_spaces_inside_the_trailing_parameter() {
        assert_eq!(parse("TOPIC #a :two  spaces ").params, ["#a", "two  spaces "]);
    }

    #[test]
    fn reads_an_empty_trailing_parameter() {
        assert_eq!(parse("TOPIC #a :").params, ["#a", ""]);
    }

    #[test]
    fn a_fifteenth_parameter_takes_the_rest() {
        let msg = parse("CMD 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16");
        assert_eq!(msg.params.len(), 15);
        assert_eq!(msg.params[14], "15 16");
    }

    #[test]
    fn steps_over_message_tags() {
        let msg = parse("@time=2026-10-20T12:00:00Z :bob PRIVMSG alice :hi");
        assert_eq!(msg.source(), Some("bob"));
        assert_eq!(msg.params, ["alice", "hi"]);
    }

    #[test]
    fn rejects_lines_without_a_command() {
        assert_eq!(Message::parse(""), Err(ParseError::NoCommand));
        assert_eq!(Message::parse(":prefix.only"), Err(ParseError::NoCommand));
        assert!(matches!(Message::parse("12 x"), Err(ParseError::BadCommand(_))));
        assert!(matches!(Message::parse("PRIV-MSG x"), Err(ParseError::BadCommand(_))));
    }

    #[test]
    fn recognises_numerics() {
        let msg = parse(":sampo.local 001 alice :Welcome");
        assert_eq!(msg.numeric(), Some(Numeric(1)));
        assert_eq!(msg.source(), Some("sampo.local"));
    }

    #[test]
    fn writes_trailing_text_with_a_colon() {
        let msg = Message::new("PRIVMSG").param("bob").trailing("hi");
        assert_eq!(msg.to_string(), "PRIVMSG bob :hi");
    }

    #[test]
    fn writes_a_colon_only_where_the_last_parameter_needs_one() {
        let msg = Message::new("MODE").param("#a").param("+o").param("bob");
        assert_eq!(msg.to_string(), "MODE #a +o bob");
        let msg = Message::new("TOPIC").param("#a").param("");
        assert_eq!(msg.to_string(), "TOPIC #a :");
        let msg = Message::new("PRIVMSG").param("#a").param(":-)");
        assert_eq!(msg.to_string(), "PRIVMSG #a ::-)");
    }

    #[test]
    fn round_trips() {
        for line in [
            ":sampo.local 353 alice = #sampo :@alice bob",
            ":bob!~bob@127.0.0.1 KICK #sampo alice :too loud",
            "PING :sampo.local",
            "JOIN #a,#b key",
        ] {
            assert_eq!(parse(line).to_string(), line);
        }
    }

    #[test]
    fn never_writes_a_line_break_from_a_parameter() {
        let msg = Message::new("PRIVMSG").param("bob").trailing("hi\r\nQUIT :gotcha");
        assert_eq!(msg.to_line(), "PRIVMSG bob :hi  QUIT :gotcha\r\n");
    }

    #[test]
    fn cuts_long_lines_at_512_bytes_on_a_character_boundary() {
        let text = "ö".repeat(400);
        let line = Message::new("PRIVMSG").param("#a").trailing(text).to_line();
        assert!(line.len() <= MAX_LINE);
        assert!(line.ends_with("\r\n"));
        assert!(line.trim_end().ends_with('ö'));
    }
}
