//! The screen, laid out as ircII laid it out: the conversation, a status
//! line in reverse video, and the input line at the foot.

use std::collections::VecDeque;
use std::io::{self, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::queue;
use crossterm::style::{ContentStyle, Print, PrintStyledContent, StyledContent};
use crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};
use unicode_width::UnicodeWidthStr;

use super::editor::Editor;
use super::render::style;
use super::text::{Line, Row, fit, sanitize, wrap};
use crate::banner;

/// How many lines of conversation are kept.
const SCROLLBACK: usize = 2000;

/// The screen, and the conversation that scrolls up it.
pub struct Screen {
    out: Box<dyn Write>,
    log: VecDeque<Line>,
    width: u16,
    height: u16,
    /// How many rows the view has been scrolled back.
    scroll: usize,
    /// Whether anything has changed since the last draw.
    dirty: bool,
    /// Whether the next draw must start from a blank screen.
    clear: bool,
    /// The clock as last drawn.
    clock: String,
    /// Whether to use colour at all: see <https://no-color.org>.
    color: bool,
}

impl Screen {
    pub fn new(out: Box<dyn Write>, width: u16, height: u16) -> Screen {
        Screen {
            out,
            log: VecDeque::new(),
            width,
            height,
            scroll: 0,
            dirty: true,
            clear: true,
            clock: String::new(),
            color: std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty()),
        }
    }

    /// Adds a line to the conversation.
    pub fn push(&mut self, line: Line) {
        if self.scroll > 0 {
            // Hold the view still while new lines arrive below it.
            self.scroll += wrap(&line, self.width.into(), style::stamp()).len();
        }
        self.log.push_back(line);
        if self.log.len() > SCROLLBACK {
            self.log.pop_front();
        }
        self.dirty = true;
    }

    /// Shows the emblem.
    pub fn banner(&mut self) {
        for row in banner::LOGO {
            self.push(Line::bare().span(row, style::bold()));
        }
        self.push(Line::bare());
    }

    /// Forgets the conversation so far.
    pub fn clear(&mut self) {
        self.log.clear();
        self.scroll = 0;
        self.redraw();
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        (self.width, self.height) = (width, height);
        self.scroll = 0;
        self.redraw();
    }

    /// Draws everything afresh at the next opportunity.
    pub fn redraw(&mut self) {
        self.clear = true;
        self.dirty = true;
    }

    /// Notes that something on screen has changed.
    pub fn touch(&mut self) {
        self.dirty = true;
    }

    /// Notes the passing of time, should the clock need redrawing.
    pub fn tick(&mut self) {
        if chrono::Local::now().format("%H:%M").to_string() != self.clock {
            self.dirty = true;
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn page_up(&mut self) {
        self.scroll += self.page();
        self.dirty = true;
    }

    pub fn page_down(&mut self) {
        self.scroll = self.scroll.saturating_sub(self.page());
        self.dirty = true;
    }

    fn page(&self) -> usize {
        usize::from(self.height).saturating_sub(3).max(1)
    }

    /// Draws the screen: conversation, status line and input line.
    pub fn draw(&mut self, status: (&str, &str), prompt: &str, editor: &Editor) -> io::Result<()> {
        self.dirty = false;
        self.clock = chrono::Local::now().format("%H:%M").to_string();
        let (width, height) = (usize::from(self.width), usize::from(self.height));
        if width < 16 || height < 3 {
            return Ok(());
        }
        let rows = self.visible_rows(height - 2, width);

        queue!(self.out, BeginSynchronizedUpdate, Hide)?;
        if std::mem::take(&mut self.clear) {
            queue!(self.out, Clear(ClearType::All))?;
        }
        for y in 0..height - 2 {
            queue!(self.out, MoveTo(0, y as u16))?;
            let used = match rows.get(y) {
                Some(row) => self.print_row(row)?,
                None => 0,
            };
            // Erasing from the last column would take its character with
            // it, so a full row is left as it is.
            if used < width {
                queue!(self.out, Clear(ClearType::UntilNewLine))?;
            }
        }

        let bar = status_bar(status, self.scroll > 0, width);
        queue!(
            self.out,
            MoveTo(0, (height - 2) as u16),
            PrintStyledContent(StyledContent::new(style::status(), bar))
        )?;

        let prompt = fit(&sanitize(prompt), width / 2);
        let indent = prompt.width();
        let (text, cursor) = editor.view(width - indent);
        queue!(
            self.out,
            MoveTo(0, (height - 1) as u16),
            Print(&prompt),
            Print(&text),
            Clear(ClearType::UntilNewLine),
            MoveTo((indent + cursor) as u16, (height - 1) as u16),
            Show,
            EndSynchronizedUpdate
        )?;
        self.out.flush()
    }

    /// Prints a row, and returns how many columns it took.
    fn print_row(&mut self, row: &Row) -> io::Result<usize> {
        let mut used = 0;
        for span in row {
            let style = if self.color {
                span.style
            } else {
                ContentStyle { foreground_color: None, background_color: None, ..span.style }
            };
            queue!(self.out, PrintStyledContent(StyledContent::new(style, span.text.as_str())))?;
            used += span.text.width();
        }
        Ok(used)
    }

    /// The rows of conversation that fit in `height` rows, top to bottom.
    fn visible_rows(&mut self, height: usize, width: usize) -> Vec<Row> {
        let wanted = height + self.scroll;
        let mut rows: Vec<Row> = Vec::new();
        for line in self.log.iter().rev() {
            rows.extend(wrap(line, width, style::stamp()).into_iter().rev());
            if rows.len() >= wanted {
                break;
            }
        }
        // Scrolling back past the first line stops at the first line.
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
        let mut shown: Vec<Row> = rows.into_iter().skip(self.scroll).take(height).collect();
        shown.reverse();
        shown
    }

    /// The conversation so far, oldest first.
    #[cfg(test)]
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.log.iter()
    }
}

/// The status line, padded or cut to the width of the screen.
fn status_bar((left, right): (&str, &str), more: bool, width: usize) -> String {
    let mut left = sanitize(left);
    if more {
        left.push_str(" -- more --");
    }
    let right = sanitize(right);
    let used = left.width() + right.width();
    if used < width {
        format!("{left}{}{right}", " ".repeat(width - used))
    } else {
        let left = fit(&left, width);
        let pad = width - left.width();
        format!("{left}{}", " ".repeat(pad))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_bar_spans_the_screen() {
        let bar = status_bar((" 12:00 alice", "sampo.local "), false, 40);
        assert_eq!(bar.width(), 40);
        assert!(bar.starts_with(" 12:00 alice") && bar.ends_with("sampo.local "));
        let bar = status_bar((" 12:00 alice on #a-very-long-channel-name", "server "), true, 20);
        assert_eq!(bar.width(), 20);
    }

    #[test]
    fn scrolling_stops_at_the_top() {
        let mut screen = Screen::new(Box::new(io::sink()), 40, 10);
        for i in 0..20 {
            screen.push(Line::now().span(&format!("line {i}"), ContentStyle::new()));
        }
        screen.page_up();
        screen.page_up();
        screen.page_up();
        let rows = screen.visible_rows(8, 40);
        assert_eq!(rows.len(), 8);
        let first: String = rows[0].iter().map(|s| s.text.as_str()).collect();
        assert!(first.ends_with("line 0"), "{first}");
        screen.draw(("", ""), "> ", &Editor::new()).unwrap();
    }

    /// What the screen writes, kept for inspection.
    #[derive(Clone, Default)]
    struct Tape(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl Write for Tape {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Just enough of a VT100 to see where things land: cursor movement,
    /// erasing, and printing. Colours and modes are ignored.
    fn replay(output: &[u8], width: usize, height: usize) -> Vec<String> {
        let mut grid = vec![vec![' '; width]; height];
        let (mut x, mut y) = (0, 0);
        let text = String::from_utf8_lossy(output);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                if y < height && x < width {
                    grid[y][x] = c;
                }
                x += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                continue;
            }
            chars.next(); // '['
            let mut params = String::new();
            while let Some(&p) = chars.peek().filter(|p| !p.is_ascii_alphabetic()) {
                params.push(p);
                chars.next();
            }
            match chars.next() {
                Some('H') => {
                    let mut numbers = params.split(';').map(|n| n.parse::<usize>().unwrap_or(1));
                    y = numbers.next().unwrap_or(1) - 1;
                    x = numbers.next().unwrap_or(1) - 1;
                }
                Some('K') if y < height => grid[y][x.min(width)..].fill(' '),
                Some('J') => grid.iter_mut().for_each(|row| row.fill(' ')),
                _ => {}
            }
        }
        grid.into_iter().map(|row| row.into_iter().collect()).collect()
    }

    #[test]
    fn the_screen_is_laid_out_as_ircii_laid_it_out() {
        let tape = Tape::default();
        let mut screen = Screen::new(Box::new(tape.clone()), 60, 12);
        screen.push(Line::now().span("*** ", style::info()).span("Connecting...", style::plain()));
        let long = "the mill grinds out grain, and salt, and gold, without end";
        screen.push(Line::now().span("<bob> ", style::plain()).span(long, style::plain()));
        let mut editor = Editor::new();
        editor.insert_str("hello");
        let status = (" 12:00 alice on #sampo(+nt)", "sampo.local ");
        screen.draw(status, "[#sampo] ", &editor).unwrap();

        let rows = replay(&tape.0.borrow(), 60, 12);
        println!("{}", rows.join("|\n"));
        if !tape.0.borrow().contains(&0x1b) {
            return; // A console without ANSI was driven directly; nothing to see here.
        }
        assert!(rows[0].trim_end().ends_with(" *** Connecting..."));
        assert!(rows[1].contains("<bob> the mill grinds out grain, and salt, and gold,"));
        assert_eq!(rows[2].trim(), "without end", "wrapped beneath the text");
        assert!(rows[3].trim().is_empty());
        assert!(rows[10].starts_with(" 12:00 alice on #sampo(+nt)"));
        assert!(rows[10].ends_with("sampo.local "));
        assert_eq!(rows[11].trim_end(), "[#sampo] hello");
    }
}
