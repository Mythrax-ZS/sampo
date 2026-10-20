//! # sampo
//!
//! An IRC server and a terminal client, in the classic style.
//!
//! ```text
//!  ▄▀▀▀▀▀▀▀▄
//! █ ▄█▄▄▄█▄ █   ▄▀▀▀▀ ▄▀▀▀▄ █▄ ▄█ █▀▀▀▄ ▄▀▀▀▄
//! █  █   █  █    ▀▀▀▄ █▀▀▀█ █ ▀ █ █▀▀▀  █   █
//! █ ▀█▀▀▀█▀ █   ▀▀▀▀  ▀   ▀ ▀   ▀ ▀      ▀▀▀
//!  ▀▄▄▄▄▄▄▄▀    internet relay chat · forged in rust
//! ```
//!
//! The crate has three parts:
//!
//! * [`proto`] is the wire protocol of RFC 1459: messages, numeric replies,
//!   case mapping and line framing, shared by both ends of a connection.
//! * [`server`] is a small, complete IRC daemon.
//! * [`client`] is a terminal client in the manner of ircII.
//!
//! The `sampo` binary puts a command line in front of the last two.

pub mod banner;
pub mod proto;
