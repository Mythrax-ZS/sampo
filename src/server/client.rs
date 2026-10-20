//! The record of one connection: who is on the other end, and how to
//! reach them.

use std::collections::BTreeSet;
use std::fmt;

use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::Instant;

use super::Line;
use crate::proto::Folded;

/// Connections are numbered from 1, and numbers are never reused, so a
/// stale id can only ever miss.
pub type ClientId = u64;

/// One connection to the server, and once it has registered, the user
/// behind it. RFC 1459 calls anything that is not a server a client.
pub struct Client {
    pub id: ClientId,
    /// The host part of the mask: an address, since there is no DNS here.
    pub host: String,
    pub nick: Option<String>,
    /// The username from USER, with the `~` that says no ident vouched for it.
    pub user: Option<String>,
    pub realname: String,
    /// What the client gave with PASS, if anything.
    pub password: Option<String>,
    /// Whether NICK and USER are done and the welcome has been sent.
    pub registered: bool,
    pub modes: UserModes,
    pub away: Option<String>,
    /// The channels this client is in.
    pub channels: BTreeSet<Folded>,
    /// When registration finished, in seconds since the epoch.
    pub signon: i64,
    pub connected: Instant,
    /// When anything at all was last heard from the client.
    pub last_heard: Instant,
    /// When the client last said something to someone, for WHOIS idle time.
    pub last_spoke: Instant,
    /// When an unanswered PING went out, if one has.
    pub ping_sent: Option<Instant>,
    /// The writer's queue: the client's SendQ.
    pub queue: mpsc::Sender<Line>,
    pub reader: AbortHandle,
    pub writer: JoinHandle<()>,
}

impl Client {
    /// A connection that has said nothing yet.
    pub fn new(
        id: ClientId,
        host: String,
        queue: mpsc::Sender<Line>,
        reader: AbortHandle,
        writer: JoinHandle<()>,
    ) -> Client {
        let now = Instant::now();
        Client {
            id,
            host,
            nick: None,
            user: None,
            realname: String::new(),
            password: None,
            registered: false,
            modes: UserModes::default(),
            away: None,
            channels: BTreeSet::new(),
            signon: 0,
            connected: now,
            last_heard: now,
            last_spoke: now,
            ping_sent: None,
            queue,
            reader,
            writer,
        }
    }

    /// The nickname, or `*` before one is chosen, as replies address such
    /// a client.
    pub fn nick(&self) -> &str {
        self.nick.as_deref().unwrap_or("*")
    }

    /// The username, or `*` before USER.
    pub fn user(&self) -> &str {
        self.user.as_deref().unwrap_or("*")
    }

    /// `nick!user@host`: the prefix on everything this client says.
    pub fn mask(&self) -> String {
        format!("{}!{}@{}", self.nick(), self.user(), self.host)
    }
}

/// The user modes: `+i` invisible, `+o` IRC operator, `+w` receives WALLOPS.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UserModes {
    pub invisible: bool,
    pub operator: bool,
    pub wallops: bool,
}

impl UserModes {
    /// Every user mode there is, as `RPL_MYINFO` lists them.
    pub const ALL: &'static str = "iow";

    /// Whether no modes are set.
    pub fn is_empty(self) -> bool {
        self == UserModes::default()
    }
}

impl fmt::Display for UserModes {
    /// Writes the modes as `+iw`, or just `+` when none are set.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("+")?;
        for (set, letter) in [(self.invisible, "i"), (self.operator, "o"), (self.wallops, "w")] {
            if set {
                f.write_str(letter)?;
            }
        }
        Ok(())
    }
}
