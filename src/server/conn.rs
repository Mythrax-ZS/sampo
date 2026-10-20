//! The pumps: tasks that move lines between a socket and the server, and
//! know nothing of IRC beyond where one line ends and the next begins.

use std::io;

use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{self, Duration, Instant};

use super::client::ClientId;
use super::{Event, Line};
use crate::proto::{LineReader, Message};

/// What each command costs a client from its allowance...
const PENALTY: Duration = Duration::from_secs(1);
/// ...and how far ahead of the clock it may spend before it must wait.
const BURST: Duration = Duration::from_secs(10);
/// A write that takes longer than this means the client has stopped reading.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Accepts connections and hands them to the server, until aborted.
pub(super) async fn accept_loop(listener: TcpListener, events: mpsc::Sender<Event>) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                if events.send(Event::Accepted(stream, addr)).await.is_err() {
                    return;
                }
            }
            Err(e) => {
                // Most likely out of file descriptors. Spinning would not
                // bring any back, so pause a moment.
                log!("cannot accept: {e}");
                time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

/// Reads a client's lines and passes them to the server as messages.
pub(super) async fn read_loop(socket: OwnedReadHalf, id: ClientId, events: mpsc::Sender<Event>) {
    let mut lines = LineReader::new(socket);
    let mut throttle = Throttle::new();
    let reason = loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                throttle.wait().await;
                // Like ircd, say nothing about lines that make no sense.
                let Ok(message) = Message::parse(&line) else {
                    continue;
                };
                if events.send(Event::Message(id, message)).await.is_err() {
                    return;
                }
            }
            Ok(None) => break String::from("Remote host closed the connection"),
            Err(e) => break format!("Read error: {}", describe(&e)),
        }
    };
    let _ = events.send(Event::Gone(id, reason)).await;
}

/// Writes a client's queued lines to its socket until the queue closes,
/// then hangs up.
pub(super) async fn write_loop(
    socket: OwnedWriteHalf,
    mut queue: mpsc::Receiver<Line>,
    id: ClientId,
    events: mpsc::Sender<Event>,
) {
    let mut out = BufWriter::new(socket);
    while let Some(line) = queue.recv().await {
        // Write whatever else is waiting too, then flush once.
        let written = time::timeout(WRITE_TIMEOUT, async {
            out.write_all(line.as_bytes()).await?;
            while let Ok(line) = queue.try_recv() {
                out.write_all(line.as_bytes()).await?;
            }
            out.flush().await
        })
        .await;
        let reason = match written {
            Ok(Ok(())) => continue,
            Ok(Err(e)) => format!("Write error: {}", describe(&e)),
            Err(_) => String::from("Write error: Connection timed out"),
        };
        let _ = events.send(Event::Gone(id, reason)).await;
        return;
    }
    let _ = time::timeout(WRITE_TIMEOUT, out.shutdown()).await;
}

/// Turns a connection away with a line of explanation.
pub(super) async fn refuse(mut stream: TcpStream, reason: &'static str) {
    let line = Message::new("ERROR").trailing(format!("Closing Link: ({reason})")).to_line();
    let _ = time::timeout(Duration::from_secs(5), async {
        stream.write_all(line.as_bytes()).await?;
        stream.shutdown().await
    })
    .await;
}

/// Paces a client's commands as ircd 2.8 did. Every command moves the
/// client's own clock forward, and when that clock runs too far ahead of
/// the real one, the client waits for the world to catch up. Meanwhile its
/// lines sit unread in the socket, so a flood only slows itself down.
struct Throttle {
    clock: Instant,
}

impl Throttle {
    fn new() -> Self {
        Throttle { clock: Instant::now() }
    }

    async fn wait(&mut self) {
        let now = Instant::now();
        self.clock = self.clock.max(now) + PENALTY;
        if self.clock > now + BURST {
            time::sleep_until(self.clock - BURST).await;
        }
    }
}

/// A socket error in the words ircd used for its exit messages.
fn describe(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::ConnectionReset => "Connection reset by peer".into(),
        io::ErrorKind::ConnectionAborted => "Software caused connection abort".into(),
        io::ErrorKind::BrokenPipe => "Broken pipe".into(),
        io::ErrorKind::TimedOut => "Connection timed out".into(),
        _ => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_burst_is_free_and_a_flood_is_paced() {
        let mut throttle = Throttle::new();
        let start = Instant::now();
        for _ in 0..10 {
            throttle.wait().await;
        }
        assert_eq!(Instant::now(), start, "the first ten commands cost nothing");
        for _ in 0..5 {
            throttle.wait().await;
        }
        assert_eq!(Instant::now() - start, Duration::from_secs(5), "then one a second");
    }
}
