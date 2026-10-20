//! The client: a terminal IRC client in the manner of ircII.
//!
//! ```text
//! 12:01 *** alice (~alice@127.0.0.1) has joined channel #sampo
//! 12:01 <bob> morning, alice
//! 12:02 > morning, bob
//!  12:02 alice on #sampo(+nt) [Act: #rust]                  sampo.local
//! [#sampo] _
//! ```
//!
//! The conversation scrolls down from the top, the status line sits under
//! it in reverse video, and you type on the line at the bottom. Everything
//! arrives in the one window, with the channel named whenever it is not the
//! one you are talking in. A line starting with `/` is a command, and any
//! command the client does not know goes to the server as it stands, just
//! as ircII did it.

mod commands;
mod editor;
mod handlers;
mod net;
mod render;
mod screen;
mod session;
mod term;
mod text;

use std::fmt;
use std::io;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{self, Instant};

use crate::proto::Message;
use editor::Editor;
use net::{Link, NetEvent, NetKind};
use screen::Screen;
use session::Session;
use term::Input;

/// What to connect to, and who to be.
#[derive(Debug, Clone)]
pub struct Options {
    pub host: String,
    pub port: u16,
    pub nick: String,
    pub user: String,
    pub realname: String,
    pub password: Option<String>,
    /// Channels to join once connected.
    pub channels: Vec<String>,
}

/// A failure that ends the client.
#[derive(Debug)]
pub enum Error {
    /// The terminal could not be taken over, or written to.
    Terminal(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Terminal(error) => write!(f, "terminal: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Terminal(error) => Some(error),
        }
    }
}

/// Reads `host`, `host:port`, `[v6 address]:port`, or a bare v6 address.
pub fn parse_address(text: &str) -> Result<(String, Option<u16>), String> {
    let port = |p: &str| p.parse::<u16>().map_err(|_| format!("{p:?} is not a port number"));
    if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or("an address with [ needs a ]")?;
        return match after {
            "" => Ok((host.to_owned(), None)),
            _ => match after.strip_prefix(':') {
                Some(p) => Ok((host.to_owned(), Some(port(p)?))),
                None => Err(format!("cannot make sense of {text:?}")),
            },
        };
    }
    match text.split_once(':') {
        Some((host, p)) if !p.contains(':') => Ok((host.to_owned(), Some(port(p)?))),
        _ => Ok((text.to_owned(), None)),
    }
}

/// Runs the client until the user quits.
pub async fn run(options: Options) -> Result<(), Error> {
    let (width, height) = crossterm::terminal::size().map_err(Error::Terminal)?;
    let terminal = term::Terminal::enter().map_err(Error::Terminal)?;
    let keyboard = term::spawn_input().map_err(Error::Terminal)?;
    let out = io::BufWriter::with_capacity(1 << 16, io::stdout());
    let mut app = App::new(options, Screen::new(Box::new(out), width, height));
    let result = app.run(keyboard).await;
    drop(terminal);
    result
}

/// The quit message, unless another is given.
const QUIT_MESSAGE: &str = "Leaving";
/// The least time between answers to CTCP requests.
const CTCP_INTERVAL: Duration = Duration::from_secs(2);

/// The client: what it knows, what it shows, and what it is connected to.
struct App {
    options: Options,
    session: Session,
    screen: Screen,
    editor: Editor,
    link: Link,
    /// News from the network tasks.
    events: mpsc::UnboundedReceiver<NetEvent>,
    sender: mpsc::UnboundedSender<NetEvent>,
    /// The connection whose news counts; news from older ones is ignored.
    generation: u64,
    quitting: bool,
    /// When a CTCP request was last answered, so that a flood of them
    /// cannot make us flood in turn.
    last_ctcp: Option<Instant>,
}

impl App {
    fn new(options: Options, screen: Screen) -> App {
        let (sender, events) = mpsc::unbounded_channel();
        App {
            session: Session::new(&options.nick, &options.channels),
            options,
            screen,
            editor: Editor::new(),
            link: Link::Down,
            events,
            sender,
            generation: 0,
            quitting: false,
            last_ctcp: None,
        }
    }

    /// The main loop: keyboard, network and clock, until the user quits.
    async fn run(&mut self, mut keyboard: mpsc::UnboundedReceiver<Input>) -> Result<(), Error> {
        self.screen.banner();
        self.connect(self.options.host.clone(), self.options.port);
        let mut clock = time::interval(Duration::from_secs(1));
        while !self.quitting {
            self.draw().map_err(Error::Terminal)?;
            tokio::select! {
                input = keyboard.recv() => {
                    let Some(input) = input else { break };
                    self.on_input(input);
                    while let Ok(input) = keyboard.try_recv() {
                        self.on_input(input);
                    }
                }
                Some(event) = self.events.recv() => {
                    self.on_net(event);
                    while let Ok(event) = self.events.try_recv() {
                        self.on_net(event);
                    }
                }
                _ = clock.tick() => self.screen.tick(),
            }
        }
        self.link.close().await;
        Ok(())
    }

    fn draw(&mut self) -> io::Result<()> {
        if !self.screen.is_dirty() {
            return Ok(());
        }
        let (left, right) = render::status(&self.session, &self.link);
        let prompt = match &self.session.current {
            Some(target) => format!("[{target}] "),
            None => String::new(),
        };
        self.screen.draw((&left, &right), &prompt, &self.editor)
    }

    // The network.

    /// Starts connecting to a server, leaving the current one if need be.
    fn connect(&mut self, host: String, port: u16) {
        self.disconnect("Changing servers");
        self.generation += 1;
        self.screen.push(render::info(&format!("Connecting to port {port} of server {host}")));
        net::connect(host.clone(), port, self.generation, self.sender.clone());
        self.link = Link::Connecting { host, port };
    }

    /// Leaves the current server, if there is one.
    fn disconnect(&mut self, reason: &str) {
        if self.link.is_up() {
            self.send(Message::new("QUIT").trailing(reason));
            self.session.disconnected();
        }
        self.link.drop_now();
    }

    fn on_net(&mut self, event: NetEvent) {
        if event.generation != self.generation {
            return;
        }
        match event.kind {
            NetKind::Connected(stream) => self.on_connected(stream),
            NetKind::Failed(error) => {
                if let Link::Connecting { host, port } =
                    std::mem::replace(&mut self.link, Link::Down)
                {
                    let why = net::describe(&error);
                    self.screen.push(render::error(&format!(
                        "Unable to connect to port {port} of server {host}: {why}"
                    )));
                    self.screen.push(render::info("Use /server to try again, or /quit to leave"));
                }
            }
            NetKind::Line(line) => self.on_line(&line),
            NetKind::Closed(reason) => {
                self.link.drop_now();
                self.session.disconnected();
                self.screen.push(render::error(&format!("Connection closed: {reason}")));
                self.screen.push(render::info("Use /server to connect again, or /quit to leave"));
            }
        }
    }

    /// The server answered: introduce ourselves.
    fn on_connected(&mut self, stream: TcpStream) {
        let Link::Connecting { host, .. } = std::mem::replace(&mut self.link, Link::Down) else {
            return;
        };
        self.link = net::start(stream, host, self.generation, self.sender.clone());
        if let Some(password) = self.options.password.clone() {
            self.send(Message::new("PASS").param(password));
        }
        self.send(Message::new("NICK").param(self.session.nick.clone()));
        let user = Message::new("USER")
            .params([self.options.user.as_str(), "0", "*"])
            .trailing(self.options.realname.clone());
        self.send(user);
    }

    /// Sends a message to the server; says so if there is no server.
    fn send(&mut self, message: Message) -> bool {
        let sent = self.link.send(message.to_line());
        if !sent {
            self.screen
                .push(render::error("You are not connected to a server. Use /server to connect"));
        }
        sent
    }

    // The keyboard.

    fn on_input(&mut self, input: Input) {
        match input {
            Input::Key(key) => self.on_key(key),
            Input::Paste(text) => self.on_paste(&text),
            Input::Resize(width, height) => self.screen.resize(width, height),
        }
        self.screen.touch();
    }

    fn on_key(&mut self, key: KeyEvent) {
        // On Windows, AltGr arrives as Ctrl and Alt together, and what it
        // types is a character like any other: the @ and {[]} of a Finnish
        // keyboard, for one.
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);
        let alt = key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
                'c' => self.quit(None),
                'd' if self.editor.is_empty() => self.quit(None),
                'd' => self.editor.delete(),
                'a' => self.editor.home(),
                'e' => self.editor.end(),
                'b' => self.editor.left(),
                'f' => self.editor.right(),
                'h' => self.editor.backspace(),
                'u' => self.editor.kill_before(),
                'k' => self.editor.kill_after(),
                'w' => self.editor.kill_word(),
                'l' => self.screen.redraw(),
                'x' | 'n' => self.cycle(1),
                'p' => self.cycle(-1),
                _ => {}
            },
            KeyCode::Char(c @ '1'..='9') if alt => {
                let index = c as usize - '1' as usize;
                if let Some(target) = self.session.targets.get(index).cloned() {
                    self.session.switch_to(&target);
                }
            }
            KeyCode::Char(c) => self.editor.insert(c),
            KeyCode::Enter => {
                let line = self.editor.submit();
                self.on_submit(&line);
            }
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Left => self.editor.left(),
            KeyCode::Right => self.editor.right(),
            KeyCode::Home => self.editor.home(),
            KeyCode::End => self.editor.end(),
            KeyCode::Up => self.editor.older(),
            KeyCode::Down => self.editor.newer(),
            KeyCode::PageUp => self.screen.page_up(),
            KeyCode::PageDown => self.screen.page_down(),
            KeyCode::Tab => self.complete(),
            _ => {}
        }
    }

    /// A paste of several lines sends each line, as if it had been typed.
    fn on_paste(&mut self, text: &str) {
        let mut lines = text.split('\n').map(|line| line.trim_end_matches('\r'));
        if let Some(first) = lines.next() {
            self.editor.insert_str(first);
        }
        for line in lines {
            let typed = self.editor.submit();
            self.on_submit(&typed);
            self.editor.insert_str(line);
        }
    }

    fn cycle(&mut self, step: isize) {
        self.session.cycle(step);
    }

    fn complete(&mut self) {
        let mut candidates = self.session.current_members();
        candidates.extend(self.session.channels.values().map(|c| c.name.clone()));
        candidates.extend(commands::NAMES.iter().map(|name| format!("/{name}")));
        self.editor.complete(&candidates);
    }

    /// Says goodbye to the server, if there is one, and ends the client.
    fn quit(&mut self, message: Option<&str>) {
        if self.link.is_up() {
            self.send(Message::new("QUIT").trailing(message.unwrap_or(QUIT_MESSAGE)));
        }
        self.quitting = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let options = Options {
            host: "localhost".into(),
            port: 6667,
            nick: "alice".into(),
            user: "alice".into(),
            realname: "Alice".into(),
            password: None,
            channels: Vec::new(),
        };
        App::new(options, Screen::new(Box::new(io::sink()), 80, 24))
    }

    /// An app with a pretend connection: what it sends arrives in the
    /// receiver.
    fn connected() -> (App, mpsc::UnboundedReceiver<String>) {
        let mut app = app();
        let (queue, sent) = mpsc::unbounded_channel();
        app.link = Link::Up {
            host: "localhost".into(),
            queue,
            reader: tokio::spawn(async {}).abort_handle(),
            writer: tokio::spawn(async {}),
        };
        (app, sent)
    }

    fn shown(app: &App) -> Vec<String> {
        app.screen.lines().map(text::Line::text).collect()
    }

    fn sent(receiver: &mut mpsc::UnboundedReceiver<String>) -> Vec<String> {
        std::iter::from_fn(|| receiver.try_recv().ok()).collect()
    }

    #[tokio::test]
    async fn a_conversation_reads_like_ircii() {
        let (mut app, mut out) = connected();
        for line in [
            ":sampo.local 001 alice :Welcome to the Sampo Internet Relay Chat Network alice!~alice@127.0.0.1",
            ":alice!~alice@127.0.0.1 JOIN #sampo",
            ":sampo.local 353 alice = #sampo :@alice bob",
            ":bob!~bob@127.0.0.1 PRIVMSG #sampo :hello alice",
            ":carol!~carol@127.0.0.1 PRIVMSG alice :psst",
            ":bob!~bob@127.0.0.1 PRIVMSG #sampo :\x01ACTION waves\x01",
            ":bob!~bob@127.0.0.1 NICK robert",
            ":robert!~bob@127.0.0.1 QUIT :Quit: bye",
            "PING :sampo.local",
        ] {
            app.on_line(line);
        }
        let text = shown(&app);
        for expected in [
            "*** alice (~alice@127.0.0.1) has joined channel #sampo",
            "*** Users on #sampo: @alice bob",
            "<bob> hello alice",
            "*carol* psst",
            "* bob waves",
            "*** bob is now known as robert",
            "*** Signoff: robert (Quit: bye)",
        ] {
            assert!(text.iter().any(|t| t == expected), "missing {expected:?} in {text:#?}");
        }
        assert!(app.session.is_current("#sampo"));
        assert_eq!(app.session.activity, ["carol"]);
        assert_eq!(sent(&mut out), ["PONG :sampo.local\r\n"]);
    }

    #[tokio::test]
    async fn typing_becomes_messages() {
        let (mut app, mut out) = connected();
        app.on_line(":alice!~alice@127.0.0.1 JOIN #sampo");
        for line in [
            "hello, world",
            "/me waves",
            "/msg bob psst",
            "/op bob carol",
            "/topic the mill grinds",
            "/motd",
            "//etc/motd is a file",
        ] {
            app.on_submit(line);
        }
        assert_eq!(
            sent(&mut out),
            [
                "PRIVMSG #sampo :hello, world\r\n",
                "PRIVMSG #sampo :\x01ACTION waves\x01\r\n",
                "PRIVMSG bob :psst\r\n",
                "MODE #sampo +oo bob carol\r\n",
                "TOPIC #sampo :the mill grinds\r\n",
                "MOTD\r\n",
                "PRIVMSG #sampo :/etc/motd is a file\r\n",
            ]
        );
        let text = shown(&app);
        assert!(text.iter().any(|t| t == "> hello, world"));
        assert!(text.iter().any(|t| t == "-> *bob* psst"));
        assert!(text.iter().any(|t| t == "* alice waves"));
    }

    #[tokio::test]
    async fn a_taken_nick_is_worked_around() {
        let (mut app, mut out) = connected();
        app.on_line(":sampo.local 433 * alice :Nickname is already in use");
        assert_eq!(sent(&mut out), ["NICK alice_\r\n"]);
        assert_eq!(app.session.nick, "alice_");
    }

    #[tokio::test]
    async fn ctcp_requests_are_answered_but_not_too_often() {
        let (mut app, mut out) = connected();
        app.on_line(":bob!~bob@127.0.0.1 PRIVMSG alice :\x01VERSION\x01");
        app.on_line(":bob!~bob@127.0.0.1 PRIVMSG alice :\x01PING 123\x01");
        let replies = sent(&mut out);
        assert_eq!(replies.len(), 1);
        assert!(replies[0].starts_with("NOTICE bob :\x01VERSION sampo "));
    }

    #[test]
    fn talking_without_a_connection_says_so() {
        let mut app = app();
        app.on_submit("/join #sampo");
        assert!(shown(&app).iter().any(|t| t.contains("not connected")));
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_address("localhost"), Ok(("localhost".into(), None)));
        assert_eq!(
            parse_address("irc.example.net:6697"),
            Ok(("irc.example.net".into(), Some(6697)))
        );
        assert_eq!(parse_address("[::1]:6667"), Ok(("::1".into(), Some(6667))));
        assert_eq!(parse_address("[::1]"), Ok(("::1".into(), None)));
        assert_eq!(parse_address("::1"), Ok(("::1".into(), None)));
        assert!(parse_address("host:port").is_err());
    }
}
