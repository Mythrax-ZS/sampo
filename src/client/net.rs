//! The connection to the server: a task to dial it, a task to read from
//! it, and a task to write to it. They report back through one channel.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, lookup_host};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time;

use crate::proto::{LineReader, MAX_LINE};

/// How long to wait for a server to answer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Something that happened on a connection. Each connection has its own
/// generation number, so that news from an abandoned one can be ignored.
#[derive(Debug)]
pub struct NetEvent {
    pub generation: u64,
    pub kind: NetKind,
}

#[derive(Debug)]
pub enum NetKind {
    /// The server answered.
    Connected(TcpStream),
    /// It did not.
    Failed(io::Error),
    /// It sent a line.
    Line(String),
    /// It hung up, or the connection failed.
    Closed(String),
}

/// The state of the connection.
pub enum Link {
    Down,
    Connecting {
        host: String,
        port: u16,
    },
    Up {
        host: String,
        queue: mpsc::UnboundedSender<String>,
        reader: AbortHandle,
        writer: JoinHandle<()>,
    },
}

impl Link {
    /// Queues a line for the server. False if there is no server.
    pub fn send(&self, line: String) -> bool {
        match self {
            Link::Up { queue, .. } => queue.send(line).is_ok(),
            _ => false,
        }
    }

    pub fn is_up(&self) -> bool {
        matches!(self, Link::Up { .. })
    }

    /// Hangs up at once. The writer still sends what it has been given.
    pub fn drop_now(&mut self) {
        if let Link::Up { reader, .. } = std::mem::replace(self, Link::Down) {
            reader.abort();
        }
    }

    /// Hangs up, giving the last lines a moment to go out.
    pub async fn close(&mut self) {
        if let Link::Up { queue, reader, writer, .. } = std::mem::replace(self, Link::Down) {
            drop(queue);
            let _ = time::timeout(Duration::from_secs(2), writer).await;
            reader.abort();
        }
    }
}

/// Dials a server in the background, and reports how it went.
pub fn connect(host: String, port: u16, generation: u64, events: mpsc::UnboundedSender<NetEvent>) {
    tokio::spawn(async move {
        let kind = match dial(&host, port).await {
            Ok(stream) => NetKind::Connected(stream),
            Err(error) => NetKind::Failed(error),
        };
        let _ = events.send(NetEvent { generation, kind });
    });
}

/// Connects to the first address for `host` that answers. IPv4 is tried
/// first: a server listening on 0.0.0.0, as sampo does by default, never
/// hears a client knocking at ::1.
async fn dial(host: &str, port: u16) -> io::Result<TcpStream> {
    let mut addrs: Vec<SocketAddr> = lookup_host((host, port)).await?.collect();
    addrs.sort_by_key(SocketAddr::is_ipv6);
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address found");
    for addr in addrs {
        match time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = error,
            Err(_) => last = io::Error::new(io::ErrorKind::TimedOut, "connection timed out"),
        }
    }
    Err(last)
}

/// Sets a fresh connection's reader and writer going.
pub fn start(
    stream: TcpStream,
    host: String,
    generation: u64,
    events: mpsc::UnboundedSender<NetEvent>,
) -> Link {
    let _ = stream.set_nodelay(true);
    let (reader, writer) = stream.into_split();
    let (queue, outbox) = mpsc::unbounded_channel();
    let writer = tokio::spawn(write_loop(writer, outbox));
    let reader = tokio::spawn(read_loop(reader, generation, events)).abort_handle();
    Link::Up { host, queue, reader, writer }
}

async fn read_loop(
    socket: OwnedReadHalf,
    generation: u64,
    events: mpsc::UnboundedSender<NetEvent>,
) {
    // Other servers may send IRCv3 tags, which can run long.
    let mut lines = LineReader::with_limit(socket, MAX_LINE * 16);
    let reason = loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let event = NetEvent { generation, kind: NetKind::Line(line) };
                if events.send(event).is_err() {
                    return;
                }
            }
            Ok(None) => break String::from("Connection closed by the server"),
            Err(error) => break describe(&error),
        }
    };
    let _ = events.send(NetEvent { generation, kind: NetKind::Closed(reason) });
}

async fn write_loop(mut socket: OwnedWriteHalf, mut outbox: mpsc::UnboundedReceiver<String>) {
    while let Some(line) = outbox.recv().await {
        if socket.write_all(line.as_bytes()).await.is_err() {
            return;
        }
    }
    let _ = socket.shutdown().await;
}

/// A socket error in the old, short words.
pub fn describe(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::ConnectionRefused => "Connection refused".into(),
        io::ErrorKind::ConnectionReset => "Connection reset by peer".into(),
        io::ErrorKind::TimedOut => "Connection timed out".into(),
        _ => error.to_string(),
    }
}
