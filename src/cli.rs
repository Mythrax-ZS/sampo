//! The command line, read by hand in the getopt manner: short options and
//! long, values attached or apart, and `--` to end the options.

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};

use sampo::banner::VERSION;
use sampo::client;
use sampo::server::{Config, DEFAULT_PORT, Oper};

/// What was asked for.
pub enum Command {
    Serve(Config),
    Connect(client::Options),
    Help,
    Version,
}

/// The help text.
pub fn usage() -> String {
    format!(
        "\
sampo {VERSION}: internet relay chat, forged in rust

Usage:
  sampo serve   [options]                 run an IRC server
  sampo connect [options] [host[:port]]   connect to one (localhost:{DEFAULT_PORT} by default)

Server options:
  -l, --listen ADDR      where to listen: a port, host:port or [v6]:port
                         (0.0.0.0:{DEFAULT_PORT} by default; may be given more than once)
  -n, --name NAME        the server's name (sampo.local by default)
      --network NAME     the network's name (Sampo by default)
      --info TEXT        a line about the server, for WHOIS
  -m, --motd FILE        the message of the day
      --password PASS    a password clients must give to connect
      --oper NAME:PASS   an IRC operator (may be given more than once)
      --max-clients N    the most connections at once (1024 by default)

Client options:
  -n, --nick NICK        your nickname (your login name by default)
  -u, --user NAME        your username (your nickname by default)
  -r, --realname TEXT    your real name, as WHOIS shows it
  -p, --password PASS    the server's password, if it has one
  -j, --join CHANNELS    channels to join, separated by commas

  -h, --help             show this help
  -V, --version          show the version"
    )
}

/// A command line that cannot be acted on.
#[derive(Debug)]
pub enum UsageError {
    /// The first argument was neither `serve` nor `connect`.
    UnknownCommand(String),
    UnknownOption(String),
    /// An option that needs a value came last.
    MissingValue(String),
    /// An option that takes no value was given one.
    UnwantedValue(String),
    BadValue {
        option: String,
        value: String,
        why: String,
    },
    UnexpectedArgument(String),
    /// The message of the day could not be read.
    Motd {
        path: String,
        error: io::Error,
    },
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsageError::UnknownCommand(command) => {
                write!(f, "unknown command '{command}' (try 'serve' or 'connect')")
            }
            UsageError::UnknownOption(option) => write!(f, "unknown option '{option}'"),
            UsageError::MissingValue(option) => write!(f, "option '{option}' needs a value"),
            UsageError::UnwantedValue(option) => write!(f, "option '{option}' takes no value"),
            UsageError::BadValue { option, value, why } => {
                write!(f, "bad value '{value}' for '{option}': {why}")
            }
            UsageError::UnexpectedArgument(arg) => write!(f, "unexpected argument '{arg}'"),
            UsageError::Motd { path, error } => write!(f, "cannot read {path}: {error}"),
        }
    }
}

impl std::error::Error for UsageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UsageError::Motd { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Reads the command line, less the program's name.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, UsageError> {
    let mut args = Args::new(args);
    let Some(first) = args.items.next() else {
        return Ok(Command::Help);
    };
    match first.as_str() {
        "serve" | "server" => serve(&mut args),
        "connect" | "client" => connect(&mut args),
        "help" | "-h" | "--help" => Ok(Command::Help),
        "version" | "-V" | "--version" => Ok(Command::Version),
        option if option.starts_with('-') => Err(UsageError::UnknownOption(first)),
        _ => Err(UsageError::UnknownCommand(first)),
    }
}

fn serve(args: &mut Args) -> Result<Command, UsageError> {
    let mut config = Config::default();
    let mut listen = Vec::new();
    while let Some(arg) = args.next() {
        let option = match arg {
            Arg::Option(option) => option,
            Arg::Positional(arg) => return Err(UsageError::UnexpectedArgument(arg)),
        };
        match option.as_str() {
            "-l" | "--listen" => {
                let value = args.value(&option)?;
                listen.extend(listen_addrs(&option, &value)?);
            }
            "-n" | "--name" => config.name = args.value(&option)?,
            "--network" => config.network = args.value(&option)?,
            "--info" => config.info = args.value(&option)?,
            "-m" | "--motd" => {
                let path = args.value(&option)?;
                let text = std::fs::read_to_string(&path)
                    .map_err(|error| UsageError::Motd { path, error })?;
                config.motd = Some(text.lines().map(String::from).collect());
            }
            "--password" => config.password = Some(args.value(&option)?),
            "--oper" => {
                let value = args.value(&option)?;
                let Some((name, password)) =
                    value.split_once(':').filter(|(n, p)| !n.is_empty() && !p.is_empty())
                else {
                    return Err(bad(&option, &value, "expected NAME:PASSWORD"));
                };
                config.opers.push(Oper { name: name.to_owned(), password: password.to_owned() });
            }
            "--max-clients" => {
                let value = args.value(&option)?;
                config.max_clients =
                    value.parse().map_err(|_| bad(&option, &value, "expected a number"))?;
            }
            "-h" | "--help" => return args.flag(&option).map(|()| Command::Help),
            _ => return Err(UsageError::UnknownOption(option)),
        }
    }
    if !listen.is_empty() {
        config.listen = listen;
    }
    Ok(Command::Serve(config))
}

fn connect(args: &mut Args) -> Result<Command, UsageError> {
    let (mut nick, mut user, mut realname, mut password) = (None, None, None, None);
    let mut channels = Vec::new();
    let mut positional = Vec::new();
    while let Some(arg) = args.next() {
        let option = match arg {
            Arg::Option(option) => option,
            Arg::Positional(arg) => {
                positional.push(arg);
                continue;
            }
        };
        match option.as_str() {
            "-n" | "--nick" => nick = Some(args.value(&option)?),
            "-u" | "--user" => user = Some(args.value(&option)?),
            "-r" | "--realname" => realname = Some(args.value(&option)?),
            "-p" | "--password" => password = Some(args.value(&option)?),
            "-j" | "--join" => {
                let value = args.value(&option)?;
                let names = value.split(',').filter(|name| !name.is_empty());
                channels.extend(names.map(|name| {
                    if name.starts_with(['#', '&']) { name.to_owned() } else { format!("#{name}") }
                }));
            }
            "-h" | "--help" => return args.flag(&option).map(|()| Command::Help),
            _ => return Err(UsageError::UnknownOption(option)),
        }
    }

    let mut positional = positional.into_iter();
    let (host, port) = match positional.next() {
        Some(address) => {
            client::parse_address(&address).map_err(|why| bad("host", &address, &why))?
        }
        None => (String::from("localhost"), None),
    };
    let port = match (port, positional.next()) {
        (Some(port), None) => port,
        (None, None) => DEFAULT_PORT,
        (None, Some(port)) => {
            port.parse().map_err(|_| bad("port", &port, "expected a port number"))?
        }
        (Some(_), Some(extra)) => return Err(UsageError::UnexpectedArgument(extra)),
    };
    if let Some(extra) = positional.next() {
        return Err(UsageError::UnexpectedArgument(extra));
    }

    let nick = nick.unwrap_or_else(default_nick);
    Ok(Command::Connect(client::Options {
        host,
        port,
        user: user.unwrap_or_else(|| nick.clone()),
        realname: realname.unwrap_or_else(|| String::from("a sampo user")),
        nick,
        password,
        channels,
    }))
}

/// Reads an address to listen on: `6667`, `:6667`, `127.0.0.1:6667`,
/// `[::]:6667` or `localhost:6667`. A port alone means every IPv4 address.
fn listen_addrs(option: &str, value: &str) -> Result<Vec<SocketAddr>, UsageError> {
    let port = value.strip_prefix(':').unwrap_or(value);
    if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
        let port = port.parse().map_err(|_| bad(option, value, "not a port number"))?;
        return Ok(vec![SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))]);
    }
    value
        .to_socket_addrs()
        .map(Iterator::collect)
        .map_err(|error| bad(option, value, &error.to_string()))
}

/// The login name, made into a nickname; failing that, `guest`.
fn default_nick() -> String {
    let login = ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default();
    let special = |c: char| "[]\\`_^{|}".contains(c);
    let nick: String = login
        .chars()
        .filter(|&c| c.is_ascii_alphanumeric() || special(c) || c == '-')
        .take(16)
        .collect();
    match nick.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || special(c) => nick,
        _ => String::from("guest"),
    }
}

fn bad(option: &str, value: &str, why: &str) -> UsageError {
    UsageError::BadValue { option: option.to_owned(), value: value.to_owned(), why: why.to_owned() }
}

/// The arguments after the command, read one at a time.
struct Args {
    items: std::vec::IntoIter<String>,
    /// A value attached to the option just read: `--name=x`, or `-nx`.
    attached: Option<String>,
    /// Set by `--`: everything after it is an argument, not an option.
    no_more_options: bool,
}

enum Arg {
    Option(String),
    Positional(String),
}

impl Args {
    fn new(args: impl IntoIterator<Item = String>) -> Args {
        Args {
            items: args.into_iter().collect::<Vec<_>>().into_iter(),
            attached: None,
            no_more_options: false,
        }
    }

    fn next(&mut self) -> Option<Arg> {
        if let Some(value) = self.attached.take() {
            // The last option did not want it.
            return Some(Arg::Positional(value));
        }
        let item = self.items.next()?;
        if self.no_more_options || item == "-" || !item.starts_with('-') {
            return Some(Arg::Positional(item));
        }
        if item == "--" {
            self.no_more_options = true;
            return self.next();
        }
        if let Some(long) = item.strip_prefix("--") {
            return Some(Arg::Option(match long.split_once('=') {
                Some((name, value)) => {
                    self.attached = Some(value.to_owned());
                    format!("--{name}")
                }
                None => item,
            }));
        }
        let mut chars = item.chars().skip(1);
        let flag = chars.next()?;
        let rest: String = chars.collect();
        if !rest.is_empty() {
            self.attached = Some(rest);
        }
        Some(Arg::Option(format!("-{flag}")))
    }

    /// The value for an option: attached to it, or the next argument.
    fn value(&mut self, option: &str) -> Result<String, UsageError> {
        self.attached
            .take()
            .or_else(|| self.items.next())
            .ok_or_else(|| UsageError::MissingValue(option.to_owned()))
    }

    /// Checks that an option that takes no value was not given one.
    fn flag(&mut self, option: &str) -> Result<(), UsageError> {
        match self.attached.take() {
            Some(_) => Err(UsageError::UnwantedValue(option.to_owned())),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_line(line: &str) -> Result<Command, UsageError> {
        parse(line.split_whitespace().map(String::from))
    }

    #[test]
    fn defaults_are_classic() {
        let Ok(Command::Serve(config)) = parse_line("serve") else { panic!() };
        assert_eq!(config.listen, [SocketAddr::from((Ipv4Addr::UNSPECIFIED, 6667))]);
        let Ok(Command::Connect(options)) = parse_line("connect -n alice") else { panic!() };
        assert_eq!((options.host.as_str(), options.port), ("localhost", 6667));
        assert_eq!(options.user, "alice");
    }

    #[test]
    fn options_come_in_every_style() {
        let line = "serve -l 7000 --listen=127.0.0.1:7001 -nirc.example.net --oper root:hunter2";
        let Ok(Command::Serve(config)) = parse_line(line) else { panic!() };
        assert_eq!(config.listen.len(), 2);
        assert_eq!(config.listen[0].port(), 7000);
        assert_eq!(config.name, "irc.example.net");
        assert_eq!(config.opers[0].password, "hunter2");
    }

    #[test]
    fn the_client_takes_an_address_and_channels() {
        let Ok(Command::Connect(options)) =
            parse_line("connect irc.example.net 7000 -j sampo,#rust")
        else {
            panic!()
        };
        assert_eq!((options.host.as_str(), options.port), ("irc.example.net", 7000));
        assert_eq!(options.channels, ["#sampo", "#rust"]);
        let Ok(Command::Connect(options)) = parse_line("connect [::1]:6697") else { panic!() };
        assert_eq!((options.host.as_str(), options.port), ("::1", 6697));
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(matches!(parse_line("dance"), Err(UsageError::UnknownCommand(_))));
        assert!(matches!(parse_line("serve --colour"), Err(UsageError::UnknownOption(_))));
        assert!(matches!(parse_line("serve -l"), Err(UsageError::MissingValue(_))));
        assert!(matches!(parse_line("serve --oper root"), Err(UsageError::BadValue { .. })));
        assert!(matches!(parse_line("serve extra"), Err(UsageError::UnexpectedArgument(_))));
        assert!(matches!(parse_line("connect a 1 c"), Err(UsageError::UnexpectedArgument(_))));
        assert!(matches!(parse_line("connect a b"), Err(UsageError::BadValue { .. })));
        assert!(matches!(parse_line("connect --help=please"), Err(UsageError::UnwantedValue(_))));
    }
}
