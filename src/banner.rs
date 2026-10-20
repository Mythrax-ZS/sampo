//! The emblem, and the words that travel with it.

/// The crate version, as Cargo knows it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The emblem: the channel sign struck into a seal, beside the name.
pub const LOGO: [&str; 5] = [
    " ▄▀▀▀▀▀▀▀▄",
    "█ ▄█▄▄▄█▄ █   ▄▀▀▀▀ ▄▀▀▀▄ █▄ ▄█ █▀▀▀▄ ▄▀▀▀▄",
    "█  █   █  █    ▀▀▀▄ █▀▀▀█ █ ▀ █ █▀▀▀  █   █",
    "█ ▀█▀▀▀█▀ █   ▀▀▀▀  ▀   ▀ ▀   ▀ ▀      ▀▀▀",
    " ▀▄▄▄▄▄▄▄▀    internet relay chat · forged in rust",
];

/// The version as a server announces it: `sampo-0.1.0`.
pub fn server_version() -> String {
    format!("sampo-{VERSION}")
}

/// The message of the day for a server that has not been given one.
pub fn motd(server: &str) -> Vec<String> {
    let mut lines: Vec<String> = LOGO.map(String::from).to_vec();
    lines.extend(
        [
            "",
            &format!("Welcome to {server}."),
            "",
            "In the Kalevala, Ilmarinen the smith forged the Sampo, a mill",
            "that ground out grain, salt and gold from nothing at all.",
            "This one grinds out conversation.",
            "",
            "Try /join #sampo, and be kind to strangers.",
        ]
        .map(String::from),
    );
    lines
}

/// What the INFO command tells anyone who asks.
pub const INFO: &[&str] = &[
    "sampo: internet relay chat, forged in rust.",
    "",
    "IRC was written by Jarkko Oikarinen at the University of Oulu,",
    "Finland, in August 1988. The first server was tolsun.oulu.fi.",
    "In May 1993 he and Darren Reed set the protocol down as RFC 1459,",
    "and this server tries to keep faith with it.",
    "",
    "The Sampo is the mill of the Kalevala, forged by the smith",
    "Ilmarinen, that ground out grain, salt and gold without end.",
    "Everyone wanted it. It broke to pieces in a battle at sea, and",
    "the pieces still bring luck to whoever finds them.",
    "",
    "For everyone who ever kept a server up at three in the morning.",
];
