//! The terminal: raw mode and the alternate screen on the way in, and
//! everything put back on the way out, panic or no panic.

use std::io;
use std::thread;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEvent, KeyEventKind,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute, style};
use tokio::sync::mpsc;

/// Something that happened at the keyboard, or to the window.
#[derive(Debug)]
pub enum Input {
    Key(KeyEvent),
    Paste(String),
    Resize(u16, u16),
}

/// The terminal, taken over. Dropping this gives it back as it was found.
pub struct Terminal(());

impl Terminal {
    /// Puts the terminal in raw mode on the alternate screen.
    pub fn enter() -> io::Result<Terminal> {
        terminal::enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = terminal::disable_raw_mode();
            return Err(e);
        }
        // Not every console knows bracketed paste. Without it, a paste
        // simply arrives as typing, a line at a time.
        let _ = execute!(io::stdout(), EnableBracketedPaste);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
        Ok(Terminal(()))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

/// Undoes everything [`Terminal::enter`] did. Each step is taken on its
/// own, so that one failing cannot stop the rest.
fn restore() {
    let mut out = io::stdout();
    let _ = execute!(out, DisableBracketedPaste);
    let _ = execute!(out, style::ResetColor);
    let _ = execute!(out, cursor::Show);
    let _ = execute!(out, LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

/// Starts a thread to read the keyboard, and returns what it reads.
///
/// Reading a terminal blocks, and there is no portable way to wait on one
/// alongside sockets, so it gets a thread of its own, as in the curses
/// programs of old.
pub fn spawn_input() -> io::Result<mpsc::UnboundedReceiver<Input>> {
    let (sender, receiver) = mpsc::unbounded_channel();
    thread::Builder::new().name(String::from("keyboard")).spawn(move || {
        loop {
            let input = match event::read() {
                // Windows reports releases as well as presses.
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => Input::Key(key),
                Ok(Event::Paste(text)) => Input::Paste(text),
                Ok(Event::Resize(width, height)) => Input::Resize(width, height),
                Ok(_) => continue,
                Err(_) => return,
            };
            if sender.send(input).is_err() {
                return;
            }
        }
    })?;
    Ok(receiver)
}
