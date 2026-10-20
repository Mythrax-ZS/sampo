//! How things look: the line formats of ircII, give or take some colour.
//!
//! ```text
//! 12:00 *** alice (~alice@10.0.0.7) has joined channel #sampo
//! 12:00 <bob> morning, alice          a message in the current channel
//! 12:01 <carol:#rust> anyone here?    a message in another channel
//! 12:01 > morning, bob                something we said
//! 12:02 *dave* psst                   a private message
//! 12:02 -> *dave* what?               a private message we sent
//! 12:03 * bob waves                   an action
//! 12:03 -sampo.local- hello           a notice
//! ```

use std::fmt::Write;

use crossterm::style::{Attribute, Color, ContentStyle};

use super::net::Link;
use super::session::Session;
use super::text::Line;
use crate::proto::Folded;

/// The styles used on screen.
pub mod style {
    use super::*;

    pub fn plain() -> ContentStyle {
        ContentStyle::new()
    }

    fn fg(color: Color) -> ContentStyle {
        ContentStyle { foreground_color: Some(color), ..ContentStyle::new() }
    }

    fn with(mut style: ContentStyle, attribute: Attribute) -> ContentStyle {
        style.attributes.set(attribute);
        style
    }

    pub fn stamp() -> ContentStyle {
        fg(Color::DarkGrey)
    }

    /// The `***` of ircII.
    pub fn info() -> ContentStyle {
        fg(Color::DarkCyan)
    }

    pub fn error() -> ContentStyle {
        fg(Color::Red)
    }

    pub fn bold() -> ContentStyle {
        with(plain(), Attribute::Bold)
    }

    pub fn dim() -> ContentStyle {
        fg(Color::DarkGrey)
    }

    /// A line that mentions us.
    pub fn highlight() -> ContentStyle {
        with(fg(Color::Yellow), Attribute::Bold)
    }

    /// The status line.
    pub fn status() -> ContentStyle {
        with(plain(), Attribute::Reverse)
    }

    /// Each nick gets a colour of its own, the same every time it appears.
    pub fn nick(nick: &str) -> ContentStyle {
        const PALETTE: [Color; 10] = [
            Color::Red,
            Color::Green,
            Color::DarkYellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::DarkRed,
            Color::DarkGreen,
            Color::DarkMagenta,
            Color::DarkCyan,
        ];
        let hash = Folded::new(nick)
            .as_str()
            .bytes()
            .fold(5381u32, |hash, byte| hash.wrapping_mul(33) ^ u32::from(byte));
        fg(PALETTE[hash as usize % PALETTE.len()])
    }
}

/// `*** text`: news from the client or the server.
pub fn info(text: &str) -> Line {
    Line::now().span("*** ", style::info()).formatted(text, style::plain())
}

/// `*** text`, in red: something went wrong.
pub fn error(text: &str) -> Line {
    Line::now().span("*** ", style::error()).formatted(text, style::plain())
}

/// `*** nick text`, with the nick in its colour: someone did something.
pub fn event(nick: &str, text: &str) -> Line {
    Line::now()
        .span("*** ", style::info())
        .span(nick, style::nick(nick))
        .formatted(text, style::plain())
}

/// `*** Signoff: nick (reason)`, as ircII said it.
pub fn signoff(nick: &str, reason: &str) -> Line {
    Line::now()
        .span("*** ", style::info())
        .span("Signoff: ", style::plain())
        .span(nick, style::nick(nick))
        .formatted(&format!(" ({reason})"), style::plain())
}

/// `<nick> text` in the current channel, `<nick:#channel> text` elsewhere.
pub fn public(nick: &str, elsewhere: Option<&str>, text: &str, mentions_us: bool) -> Line {
    let name = if mentions_us { style::highlight() } else { style::nick(nick) };
    let mut line = Line::now().span("<", style::dim()).span(nick, name);
    if let Some(channel) = elsewhere {
        line = line.span(":", style::dim()).span(channel, style::plain());
    }
    line.span("> ", style::dim()).formatted(text, style::plain())
}

/// `> text`: something we said where we are.
pub fn own_public(text: &str) -> Line {
    Line::now().span("> ", style::bold()).formatted(text, style::plain())
}

/// `*nick* text`: a private message.
pub fn private(nick: &str, text: &str) -> Line {
    Line::now()
        .span("*", style::dim())
        .span(nick, style::nick(nick))
        .span("* ", style::dim())
        .formatted(text, style::plain())
}

/// `-> *target* text`: something we sent somewhere other than where we are.
pub fn own_private(target: &str, text: &str) -> Line {
    Line::now()
        .span("-> *", style::dim())
        .span(target, style::bold())
        .span("* ", style::dim())
        .formatted(text, style::plain())
}

/// `-nick- text`, or `-nick:#channel- text`: a notice.
pub fn notice(nick: &str, channel: Option<&str>, text: &str) -> Line {
    let mut line = Line::now().span("-", style::dim()).span(nick, style::nick(nick));
    if let Some(channel) = channel {
        line = line.span(":", style::dim()).span(channel, style::plain());
    }
    line.span("- ", style::dim()).formatted(text, style::plain())
}

/// `-> -target- text`: a notice we sent.
pub fn own_notice(target: &str, text: &str) -> Line {
    Line::now()
        .span("-> -", style::dim())
        .span(target, style::bold())
        .span("- ", style::dim())
        .formatted(text, style::plain())
}

/// `* nick waves`, or `* nick:#channel waves`: an action.
pub fn action(nick: &str, elsewhere: Option<&str>, text: &str, mentions_us: bool) -> Line {
    let name = if mentions_us { style::highlight() } else { style::nick(nick) };
    let mut line = Line::now().span("* ", style::bold()).span(nick, name);
    if let Some(channel) = elsewhere {
        line = line.span(":", style::dim()).span(channel, style::plain());
    }
    line.span(" ", style::plain()).formatted(text, style::plain())
}

/// `!nick! text`: a WALLOPS.
pub fn wallops(nick: &str, text: &str) -> Line {
    Line::now()
        .span("!", style::dim())
        .span(nick, style::nick(nick))
        .span("! ", style::dim())
        .formatted(text, style::plain())
}

/// A line of the message of the day, unadorned, so that art stays art.
pub fn motd(text: &str) -> Line {
    Line::now().formatted(text, style::plain())
}

/// The status line, in two halves: ` 12:34 alice(+i) on #sampo(+nt)
/// [Act: #rust]` on the left, and the server on the right.
pub fn status(session: &Session, link: &Link) -> (String, String) {
    // Writing to a String cannot fail.
    let mut left = format!(" {} ", chrono::Local::now().format("%H:%M"));
    let sign = session.current_channel().and_then(|c| session.my_prefix(c));
    left.extend(sign);
    left.push_str(&session.nick);
    if !session.modes.is_empty() {
        let _ = write!(left, "(+{})", session.modes);
    }
    if session.away {
        left.push_str(" (zZzZ)");
    }
    match session.current.as_deref() {
        Some(channel) if session.is_channel(channel) => {
            let _ = write!(left, " on {channel}");
            if let Some(modes) =
                session.channel(channel).map(|c| &c.modes).filter(|m| !m.is_empty())
            {
                let _ = write!(left, "(+{modes})");
            }
        }
        Some(nick) => {
            let _ = write!(left, " talking to {nick}");
        }
        None => {}
    }
    if !session.activity.is_empty() {
        let _ = write!(left, " [Act: {}]", session.activity.join(","));
    }
    let right = match link {
        Link::Up { host, .. } => session.server.clone().unwrap_or_else(|| host.clone()),
        Link::Connecting { host, .. } => format!("connecting to {host}"),
        Link::Down => String::from("not connected"),
    };
    (left, format!("{right} "))
}
