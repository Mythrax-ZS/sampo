//! Styled text: spans of it, mIRC's formatting codes, and wrapping it to
//! the width of the screen.

use std::iter::Peekable;
use std::str::Chars;

use crossterm::style::{Attribute, Color, ContentStyle};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A run of text in one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: ContentStyle,
}

/// One line of conversation: when it happened, and what was said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    /// The time, `HH:MM`, or nothing for lines outside time, like the banner.
    pub stamp: String,
    pub spans: Vec<Span>,
}

/// A row of the screen: spans that together fit its width.
pub type Row = Vec<Span>;

impl Line {
    /// An empty line, stamped with the time now.
    pub fn now() -> Line {
        Line { stamp: chrono::Local::now().format("%H:%M").to_string(), spans: Vec::new() }
    }

    /// An empty line without a time stamp.
    pub fn bare() -> Line {
        Line::default()
    }

    /// Appends text in a style. Control characters are dropped, whatever
    /// the server sent: nothing reaches the terminal that could command it.
    #[must_use]
    pub fn span(mut self, text: &str, style: ContentStyle) -> Line {
        let text = sanitize(text);
        if !text.is_empty() {
            self.spans.push(Span { text, style });
        }
        self
    }

    /// Appends text that may carry mIRC formatting codes, over a base style.
    #[must_use]
    pub fn formatted(mut self, text: &str, base: ContentStyle) -> Line {
        self.spans.extend(formatted(text, base));
        self
    }

    /// The text of the line, without style or stamp.
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

/// Text with every control character taken out, and tabs made spaces.
pub fn sanitize(text: &str) -> String {
    text.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect()
}

/// Cuts text to at most `width` columns.
pub fn fit(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

/// Splits text carrying mIRC's formatting codes into styled spans: `^B`
/// bold, `^C` colour, `^]` italic, `^_` underline, `^V` reverse and `^O`
/// plain again. Every other control character is dropped.
pub fn formatted(text: &str, base: ContentStyle) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut style = base;
    let mut run = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let next = match c {
            '\x02' => toggled(style, Attribute::Bold),
            '\x1d' => toggled(style, Attribute::Italic),
            '\x1f' => toggled(style, Attribute::Underlined),
            '\x16' => toggled(style, Attribute::Reverse),
            '\x0f' => base,
            '\x03' => recoloured(style, base, &mut chars),
            '\t' => {
                run.push(' ');
                continue;
            }
            c if c.is_control() => continue,
            c => {
                run.push(c);
                continue;
            }
        };
        if !run.is_empty() {
            spans.push(Span { text: std::mem::take(&mut run), style });
        }
        style = next;
    }
    if !run.is_empty() {
        spans.push(Span { text: run, style });
    }
    spans
}

fn toggled(mut style: ContentStyle, attribute: Attribute) -> ContentStyle {
    style.attributes.toggle(attribute);
    style
}

/// The style after a `^C`: `^C4` red text, `^C4,1` red on black, and a
/// bare `^C` back to the base colours.
fn recoloured(
    mut style: ContentStyle,
    base: ContentStyle,
    chars: &mut Peekable<Chars>,
) -> ContentStyle {
    let Some(fg) = digits(chars) else {
        style.foreground_color = base.foreground_color;
        style.background_color = base.background_color;
        return style;
    };
    style.foreground_color = mirc_color(fg).or(base.foreground_color);
    // A comma only starts a background colour if digits follow it.
    let mut ahead = chars.clone();
    if ahead.next() == Some(',') && ahead.peek().is_some_and(char::is_ascii_digit) {
        chars.next();
        if let Some(bg) = digits(chars) {
            style.background_color = mirc_color(bg).or(base.background_color);
        }
    }
    style
}

/// Up to two digits, as a number.
fn digits(chars: &mut Peekable<Chars>) -> Option<u8> {
    let mut value = None;
    for _ in 0..2 {
        let Some(digit) = chars.peek().and_then(|c| c.to_digit(10)) else {
            break;
        };
        chars.next();
        value = Some(value.unwrap_or(0) * 10 + digit as u8);
    }
    value
}

/// mIRC's sixteen colours, as near as a terminal comes.
fn mirc_color(code: u8) -> Option<Color> {
    Some(match code {
        0 => Color::White,
        1 => Color::Black,
        2 => Color::DarkBlue,
        3 => Color::DarkGreen,
        4 => Color::Red,
        5 => Color::DarkRed,
        6 => Color::DarkMagenta,
        7 => Color::DarkYellow,
        8 => Color::Yellow,
        9 => Color::Green,
        10 => Color::DarkCyan,
        11 => Color::Cyan,
        12 => Color::Blue,
        13 => Color::Magenta,
        14 => Color::DarkGrey,
        15 => Color::Grey,
        _ => return None,
    })
}

/// Breaks a line into rows no wider than `width` columns. The time stamp
/// comes first; later rows are indented to line up beneath the text, and
/// break at a space where there is one.
pub fn wrap(line: &Line, width: usize, stamp_style: ContentStyle) -> Vec<Row> {
    let stamp = !line.stamp.is_empty() && line.stamp.width() + 9 <= width;
    let indent = if stamp { line.stamp.width() + 1 } else { 0 };
    let room = width.saturating_sub(indent).max(1);

    let cells: Vec<(char, ContentStyle, usize)> = line
        .spans
        .iter()
        .flat_map(|span| span.text.chars().map(move |c| (c, span.style, c.width().unwrap_or(0))))
        .collect();

    let mut rows = Vec::new();
    let mut start = 0;
    loop {
        let mut end = start;
        let mut used = 0;
        while end < cells.len() && used + cells[end].2 <= room {
            used += cells[end].2;
            end += 1;
        }
        if end == start && end < cells.len() {
            end += 1; // Wider than the whole row: it goes in regardless.
        }
        if end < cells.len()
            && let Some(space) = (start + 1..end).rev().find(|&i| cells[i].0 == ' ')
        {
            end = space + 1;
        }

        let mut row = Row::new();
        if rows.is_empty() && stamp {
            row.push(Span { text: format!("{} ", line.stamp), style: stamp_style });
        } else if indent > 0 {
            row.push(Span { text: " ".repeat(indent), style: ContentStyle::new() });
        }
        for &(c, style, _) in &cells[start..end] {
            match row.last_mut() {
                Some(span) if span.style == style => span.text.push(c),
                _ => row.push(Span { text: c.to_string(), style }),
            }
        }
        rows.push(row);
        start = end;
        if start >= cells.len() {
            return rows;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|row| row.iter().map(|span| span.text.as_str()).collect()).collect()
    }

    fn line(text: &str) -> Line {
        Line {
            stamp: "12:34".into(),
            spans: vec![Span { text: text.into(), style: ContentStyle::new() }],
        }
    }

    #[test]
    fn short_lines_fit_on_one_row() {
        let rows = wrap(&line("hello"), 40, ContentStyle::new());
        assert_eq!(texts(&rows), ["12:34 hello"]);
    }

    #[test]
    fn long_lines_wrap_at_spaces_under_the_text() {
        let rows = wrap(&line("the quick brown fox jumps over"), 20, ContentStyle::new());
        assert_eq!(texts(&rows), ["12:34 the quick ", "      brown fox ", "      jumps over"]);
    }

    #[test]
    fn long_words_are_broken_where_they_must_be() {
        let rows = wrap(&line("abcdefghijklmnopqrstuvwxyz"), 16, ContentStyle::new());
        assert_eq!(texts(&rows), ["12:34 abcdefghij", "      klmnopqrst", "      uvwxyz"]);
    }

    #[test]
    fn wide_characters_take_two_columns() {
        let rows = wrap(&line("日本語のテキスト"), 16, ContentStyle::new());
        assert_eq!(texts(&rows), ["12:34 日本語のテ", "      キスト"]);
    }

    #[test]
    fn mirc_codes_become_styles() {
        let spans = formatted("a\x02b\x02c\x034,1d\x0fe", ContentStyle::new());
        let words: Vec<&str> = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(words, ["a", "b", "c", "d", "e"]);
        assert!(spans[1].style.attributes.has(Attribute::Bold));
        assert!(!spans[2].style.attributes.has(Attribute::Bold));
        assert_eq!(spans[3].style.foreground_color, Some(Color::Red));
        assert_eq!(spans[3].style.background_color, Some(Color::Black));
        assert_eq!(spans[4].style, ContentStyle::new());
    }

    #[test]
    fn a_comma_without_digits_is_just_a_comma() {
        let spans = formatted("\x034,hi", ContentStyle::new());
        assert_eq!(spans[0].text, ",hi");
        assert_eq!(spans[0].style.background_color, None);
    }

    #[test]
    fn terminal_escapes_never_get_through() {
        let spans = formatted("hi\x1b[2Jthere\u{9b}31m", ContentStyle::new());
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "hi[2Jthere31m");
        assert_eq!(Line::now().span("a\x1bb", ContentStyle::new()).text(), "ab");
    }
}
