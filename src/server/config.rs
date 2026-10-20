//! How a server is configured.

use std::net::{Ipv4Addr, SocketAddr};

use super::Error;

/// The port IRC has called home since 1988.
pub const DEFAULT_PORT: u16 = 6667;

/// How a server presents itself, and whom it lets in.
#[derive(Debug, Clone)]
pub struct Config {
    /// Where to listen for connections.
    pub listen: Vec<SocketAddr>,
    /// The server's name, as it appears at the head of every reply. It
    /// must contain a dot, which is how a prefix tells servers from nicks.
    pub name: String,
    /// The network's name, given in the welcome and in `RPL_ISUPPORT`.
    pub network: String,
    /// One line about the server, shown by WHOIS.
    pub info: String,
    /// The message of the day, line by line. `None` means the built-in one,
    /// and an empty list means none at all.
    pub motd: Option<Vec<String>>,
    /// A password that clients must give with PASS, if any.
    pub password: Option<String>,
    /// Who may become an IRC operator with OPER.
    pub opers: Vec<Oper>,
    /// The most connections served at once.
    pub max_clients: usize,
}

/// An operator's credentials: an O-line, in the old parlance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oper {
    /// The name given to OPER.
    pub name: String,
    /// The password given to OPER.
    pub password: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: vec![SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_PORT))],
            name: String::from("sampo.local"),
            network: String::from("Sampo"),
            info: String::from("a sampo server"),
            motd: None,
            password: None,
            opers: Vec::new(),
            max_clients: 1024,
        }
    }
}

impl Config {
    /// Checks that the configuration describes a server that can run.
    pub(super) fn validate(&self) -> Result<(), Error> {
        let bad = |why: String| Err(Error::Config(why));
        if self.listen.is_empty() {
            return bad("no address to listen on".into());
        }
        let name_ok = self.name.contains('.')
            && self.name.len() <= 63
            && self.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if !name_ok {
            return bad(format!("server name {:?} must be a hostname with a dot in it", self.name));
        }
        if self.network.is_empty()
            || self.network.contains(|c: char| c.is_whitespace() || c.is_control())
        {
            return bad(format!("network name {:?} must be a single word", self.network));
        }
        if self.max_clients == 0 {
            return bad("the client limit must be at least 1".into());
        }
        Ok(())
    }
}
