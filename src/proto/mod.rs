//! The wire protocol of RFC 1459, shared by server and client.
//!
//! IRC is lines of text over TCP. Each line is one message of no more than
//! 512 bytes, the CR-LF at its end included: an optional prefix naming the
//! sender, a command, and up to fifteen parameters.
//!
//! ```text
//! :alice!~alice@10.0.0.7 PRIVMSG #sampo :hello, world
//! ```
//!
//! That is the whole of it. The modules here handle the parts that both
//! ends need: [`message`] reads and writes those lines, [`numeric`] names
//! the three-digit replies, [`casemap`] decides when two names are the same
//! name, and [`line`](mod@line) cuts a byte stream into lines.

pub mod casemap;
pub mod line;
pub mod message;
pub mod numeric;

pub use casemap::Folded;
pub use line::LineReader;
pub use message::{Message, ParseError};
pub use numeric::Numeric;

/// The longest a message may be on the wire, CR-LF included.
pub const MAX_LINE: usize = 512;
