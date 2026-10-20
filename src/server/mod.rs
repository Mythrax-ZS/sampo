//! The server: one task that holds the whole state of the network, fed by
//! small tasks that do nothing but move lines between sockets and queues.
//!
//! The original ircd was a single process wrapped around a single
//! `select()` loop, and it carried thousands of users on machines slower
//! than a wristwatch. sampo keeps that shape. Each connection has a reader
//! task and a writer task (see `conn`), but every decision, every NICK,
//! JOIN and PRIVMSG, is made in one place, one message at a time, with no
//! locks anywhere. [`Server::run`] is that loop.
//!
//! ```text
//!   socket ─▶ reader ─┐                     ┌─▶ queue ─▶ writer ─▶ socket
//!   socket ─▶ reader ─┼─▶ events ─▶ Server ─┼─▶ queue ─▶ writer ─▶ socket
//!   listener ─────────┘                     └─▶ queue ─▶ writer ─▶ socket
//! ```

/// Writes a timestamped line to standard error.
macro_rules! log {
    ($($arg:tt)*) => {
        eprintln!(
            "{} {}",
            ::chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            format_args!($($arg)*)
        )
    };
}

mod channel;
mod client;
mod commands;
mod config;
mod conn;
mod error;

pub use config::{Config, DEFAULT_PORT, Oper};
pub use error::{Error, ErrorReply};

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{self, Instant, MissedTickBehavior};

use crate::proto::{Folded, Message, Numeric};
use channel::Channel;
use client::{Client, ClientId};

/// The longest nickname, in bytes.
pub const NICKLEN: usize = 16;
/// The longest username, `~` included.
pub const USERLEN: usize = 10;
/// The longest real name.
pub const REALLEN: usize = 50;
/// The longest channel name, `#` included.
pub const CHANNELLEN: usize = 50;
/// The longest topic.
pub const TOPICLEN: usize = 300;
/// The longest kick reason.
pub const KICKLEN: usize = 300;
/// The longest away message.
pub const AWAYLEN: usize = 200;
/// The longest channel key.
pub const KEYLEN: usize = 23;
/// How many channels one client may be in.
pub const MAX_CHANNELS: usize = 20;
/// How many targets one PRIVMSG or NOTICE may name.
pub const MAX_TARGETS: usize = 4;
/// How many mode changes with a parameter one MODE may make.
pub const MAX_MODES: usize = 4;
/// How many bans a channel may hold.
pub const MAX_BANS: usize = 50;

/// How often the clock sweeps for idle and dead connections.
const TICK: Duration = Duration::from_secs(10);
/// Silence after which a client is sent a PING.
const PING_INTERVAL: Duration = Duration::from_secs(120);
/// How long a PING may go unanswered.
const PING_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a new connection has to introduce itself.
const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(60);
/// Lines queued for one client before it is declared hopeless.
const SENDQ: usize = 1024;
/// Events queued for the server before the readers are made to wait.
const EVENT_QUEUE: usize = 1024;
/// How long shutdown waits for the last words to reach their sockets.
const LINGER: Duration = Duration::from_secs(3);

/// A message serialised for the wire, shared by all its recipients.
type Line = Arc<str>;

/// What the connection tasks tell the server.
enum Event {
    /// A listener accepted a connection.
    Accepted(TcpStream, SocketAddr),
    /// A client sent a message.
    Message(ClientId, Message),
    /// A client's socket closed or failed, for the reason given.
    Gone(ClientId, String),
}

/// An IRC server: bound to its sockets by [`Server::bind`], then brought to
/// life by [`Server::run`].
pub struct Server {
    config: Config,
    listeners: Vec<TcpListener>,
    clients: BTreeMap<ClientId, Client>,
    nicks: HashMap<Folded, ClientId>,
    channels: BTreeMap<Folded, Channel>,
    events: mpsc::Receiver<Event>,
    sender: mpsc::Sender<Event>,
    last_id: ClientId,
    /// When the server started, as RPL_CREATED puts it.
    created: String,
    /// Clients found dead while being written to. They are disconnected
    /// once the event at hand is finished, as ircd did with dead sockets.
    doomed: RefCell<Vec<(ClientId, &'static str)>>,
    /// Set by DIE: the reason the server is going down.
    dying: Option<String>,
}

impl Server {
    /// Opens the listening sockets. Nobody is served until [`Server::run`].
    pub async fn bind(config: Config) -> Result<Server, Error> {
        config.validate()?;
        let mut listeners = Vec::with_capacity(config.listen.len());
        for &addr in &config.listen {
            let listener =
                TcpListener::bind(addr).await.map_err(|source| Error::Bind { addr, source })?;
            listeners.push(listener);
        }
        let (sender, events) = mpsc::channel(EVENT_QUEUE);
        Ok(Server {
            config,
            listeners,
            clients: BTreeMap::new(),
            nicks: HashMap::new(),
            channels: BTreeMap::new(),
            events,
            sender,
            last_id: 0,
            created: chrono::Utc::now().format("%a %b %-d %Y at %H:%M:%S UTC").to_string(),
            doomed: RefCell::new(Vec::new()),
            dying: None,
        })
    }

    /// The addresses the server is listening on: useful when one was
    /// bound to port 0.
    pub fn local_addrs(&self) -> Vec<SocketAddr> {
        self.listeners.iter().filter_map(|listener| listener.local_addr().ok()).collect()
    }

    /// Serves clients until `shutdown` completes or an operator sends DIE.
    ///
    /// Shutting down closes the listening sockets first, so the port is
    /// free again at once, then bids every client farewell and waits a
    /// moment for the words to drain.
    pub async fn run(mut self, shutdown: impl Future<Output = ()>) {
        log!("{} {} starting", self.config.name, crate::banner::server_version());
        for addr in self.local_addrs() {
            log!("listening on {addr}");
        }
        let acceptors: Vec<_> = std::mem::take(&mut self.listeners)
            .into_iter()
            .map(|listener| tokio::spawn(conn::accept_loop(listener, self.sender.clone())))
            .collect();
        let mut clock = time::interval(TICK);
        clock.set_missed_tick_behavior(MissedTickBehavior::Delay);
        tokio::pin!(shutdown);

        let reason = loop {
            tokio::select! {
                Some(event) = self.events.recv() => self.handle(event),
                _ = clock.tick() => self.check_timeouts(),
                () = &mut shutdown => break String::from("Server shutting down"),
            }
            self.reap();
            if let Some(reason) = self.dying.take() {
                break reason;
            }
        };

        log!("{reason}");
        for acceptor in &acceptors {
            acceptor.abort();
        }
        for acceptor in acceptors {
            let _ = acceptor.await;
        }
        self.close_all(&reason).await;
        log!("shut down cleanly");
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Accepted(stream, addr) => self.accept(stream, addr),
            Event::Message(id, message) => self.dispatch(id, message),
            Event::Gone(id, reason) => self.exit_client(id, &reason),
        }
    }

    /// Takes on a new connection, and sets its pumps running.
    fn accept(&mut self, stream: TcpStream, addr: SocketAddr) {
        if self.clients.len() >= self.config.max_clients {
            log!("refused {addr}: server full");
            tokio::spawn(conn::refuse(stream, "Server is full"));
            return;
        }
        let _ = stream.set_nodelay(true);
        self.last_id += 1;
        let id = self.last_id;
        let host = hostname(addr.ip());
        let (reader, writer) = stream.into_split();
        let (queue, outbox) = mpsc::channel(SENDQ);
        let writer = tokio::spawn(conn::write_loop(writer, outbox, id, self.sender.clone()));
        let reader = tokio::spawn(conn::read_loop(reader, id, self.sender.clone()));
        let client = Client::new(id, host, queue, reader.abort_handle(), writer);
        let greeting = format!("*** Hello {}, no ident or hostname lookups here", client.host);
        self.clients.insert(id, client);
        self.send(id, self.server_notice("AUTH", greeting));
        log!("#{id} connected from {addr}");
    }

    /// Disconnects a client, and tells everyone who needs to know.
    /// After `exit_client()` in ircd's s_misc.c.
    fn exit_client(&mut self, id: ClientId, reason: &str) {
        let Some(client) = self.clients.get(&id) else {
            return;
        };
        if client.registered {
            let quit = Message::new("QUIT").with_prefix(client.mask()).trailing(reason);
            let quit = Line::from(quit.to_line());
            for peer in self.peers(id) {
                self.send_line(peer, &quit);
            }
        }
        let farewell = format!("Closing Link: {} ({reason})", client.host);
        self.send(id, Message::new("ERROR").trailing(farewell));

        let Some(client) = self.clients.remove(&id) else {
            return;
        };
        for key in &client.channels {
            if let Some(channel) = self.channels.get_mut(key) {
                channel.members.remove(&id);
                if channel.members.is_empty() {
                    self.channels.remove(key);
                }
            }
        }
        for channel in self.channels.values_mut() {
            channel.invites.remove(&id);
        }
        if let Some(nick) = &client.nick {
            let key = Folded::new(nick);
            if self.nicks.get(&key) == Some(&id) {
                self.nicks.remove(&key);
            }
        }
        client.reader.abort();
        log!("#{id} {} exited ({reason})", client.mask());
        // Dropping the client closes its queue; the writer sends what is
        // left, the farewell last of all, and hangs up.
    }

    /// Disconnects the clients found dead while sending.
    fn reap(&mut self) {
        loop {
            let doomed = std::mem::take(&mut *self.doomed.borrow_mut());
            if doomed.is_empty() {
                return;
            }
            for (id, reason) in doomed {
                self.exit_client(id, reason);
            }
        }
    }

    /// Pings the quiet, and drops those who never answer or never
    /// finished introducing themselves.
    fn check_timeouts(&mut self) {
        let now = Instant::now();
        let mut ping = Vec::new();
        let mut dead = Vec::new();
        for client in self.clients.values_mut() {
            if !client.registered {
                if now - client.connected > REGISTRATION_TIMEOUT {
                    dead.push((client.id, String::from("Registration timed out")));
                }
                continue;
            }
            match client.ping_sent {
                Some(sent) if now - sent > PING_TIMEOUT => {
                    let silent = (now - client.last_heard).as_secs();
                    dead.push((client.id, format!("Ping timeout: {silent} seconds")));
                }
                None if now - client.last_heard > PING_INTERVAL => {
                    client.ping_sent = Some(now);
                    ping.push(client.id);
                }
                _ => {}
            }
        }
        for id in ping {
            self.send(id, Message::new("PING").trailing(&self.config.name));
        }
        for (id, reason) in dead {
            self.exit_client(id, &reason);
        }
    }

    /// Bids every client farewell at shutdown, and waits for the words to
    /// drain from their queues.
    async fn close_all(&mut self, reason: &str) {
        let mut writers = Vec::with_capacity(self.clients.len());
        for (_, client) in std::mem::take(&mut self.clients) {
            let farewell = format!("Closing Link: {} ({reason})", client.host);
            let farewell = Message::new("ERROR").trailing(farewell).to_line();
            let _ = client.queue.try_send(Line::from(farewell));
            client.reader.abort();
            writers.push(client.writer);
        }
        self.nicks.clear();
        self.channels.clear();
        let drained = async {
            for writer in writers {
                let _ = writer.await;
            }
        };
        let _ = time::timeout(LINGER, drained).await;
    }

    // Looking things up.

    /// The client behind `id`. Only for the sender of the command being
    /// handled, which is always connected while its command runs.
    fn client(&self, id: ClientId) -> &Client {
        &self.clients[&id]
    }

    /// The client behind `id`, to change. The same caveat applies.
    fn client_mut(&mut self, id: ClientId) -> &mut Client {
        self.clients.get_mut(&id).expect("a command's sender is connected while it runs")
    }

    /// A registered client, by nickname.
    fn find_nick(&self, nick: &str) -> Option<&Client> {
        let id = self.nicks.get(&Folded::new(nick))?;
        self.clients.get(id).filter(|client| client.registered)
    }

    /// Everyone who shares a channel with `id`, each once, not counting
    /// `id` itself.
    fn peers(&self, id: ClientId) -> BTreeSet<ClientId> {
        let mut peers = BTreeSet::new();
        if let Some(client) = self.clients.get(&id) {
            for key in &client.channels {
                if let Some(channel) = self.channels.get(key) {
                    peers.extend(channel.members.keys().copied());
                }
            }
        }
        peers.remove(&id);
        peers
    }

    /// Whether `viewer` may see `target` in WHO and NAMES: themselves, the
    /// visible, and the invisible who share a channel with them.
    fn can_see(&self, viewer: ClientId, target: &Client) -> bool {
        viewer == target.id
            || !target.modes.invisible
            || self
                .clients
                .get(&viewer)
                .is_some_and(|v| v.channels.iter().any(|key| target.channels.contains(key)))
    }

    // Sending things.

    /// Queues a line for a client. One whose queue is full has stopped
    /// reading, and is marked for disconnection.
    fn send_line(&self, id: ClientId, line: &Line) {
        let Some(client) = self.clients.get(&id) else {
            return;
        };
        if let Err(mpsc::error::TrySendError::Full(_)) = client.queue.try_send(Line::clone(line)) {
            self.doomed.borrow_mut().push((id, "Max SendQ exceeded"));
        }
    }

    /// Queues a message for a client.
    fn send(&self, id: ClientId, message: Message) {
        self.send_line(id, &Line::from(message.to_line()));
    }

    /// Sends a line to every member of a channel, save perhaps one.
    fn send_channel(&self, channel: &Channel, line: &Line, except: Option<ClientId>) {
        for &member in channel.members.keys() {
            if Some(member) != except {
                self.send_line(member, line);
            }
        }
    }

    /// Starts a numeric reply to `id`: `:server 001 nick ...`.
    fn reply(&self, id: ClientId, numeric: Numeric) -> Message {
        let nick = self.clients.get(&id).map_or("*", Client::nick);
        Message::new(numeric.to_string()).with_prefix(&self.config.name).param(nick)
    }

    /// Sends an error reply to `id`.
    fn send_error(&self, id: ClientId, error: ErrorReply) {
        let (numeric, params, text) = error.parts();
        self.send(id, self.reply(id, numeric).params(params).trailing(text));
    }

    /// A notice from the server itself.
    fn server_notice(&self, target: &str, text: impl Into<String>) -> Message {
        Message::new("NOTICE").with_prefix(&self.config.name).param(target).trailing(text)
    }
}

/// The host part of a client's mask, which here is just its address.
///
/// There is no reverse DNS. A v4 address mapped into v6 is shown as plain
/// v4, and an address that starts with a colon gets a zero in front, as
/// ircd always did: `::1` as a parameter would read as a trailing one.
fn hostname(ip: IpAddr) -> String {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    let text = ip.to_string();
    if text.starts_with(':') { format!("0{text}") } else { text }
}

/// Resolves when the process is asked to stop: Ctrl-C anywhere, SIGTERM on
/// Unix, or the console window closing on Windows.
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => tokio::select! {
                () = ctrl_c() => {}
                _ = terminate.recv() => {}
            },
            Err(_) => ctrl_c().await,
        }
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows;
        match (windows::ctrl_break(), windows::ctrl_close()) {
            (Ok(mut ctrl_break), Ok(mut close)) => tokio::select! {
                () = ctrl_c() => {}
                _ = ctrl_break.recv() => {}
                _ = close.recv() => {}
            },
            _ => ctrl_c().await,
        }
    }
    #[cfg(not(any(unix, windows)))]
    ctrl_c().await;
}

/// Ctrl-C, or never, should the handler fail to install: a broken signal
/// handler must not take the server down with it.
async fn ctrl_c() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_are_addresses_made_safe_for_parameters() {
        assert_eq!(hostname("10.0.0.7".parse().unwrap()), "10.0.0.7");
        assert_eq!(hostname("::1".parse().unwrap()), "0::1");
        assert_eq!(hostname("::ffff:10.0.0.7".parse().unwrap()), "10.0.0.7");
        assert_eq!(hostname("2001:db8::1".parse().unwrap()), "2001:db8::1");
    }
}
