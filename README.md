```text
 ▄▀▀▀▀▀▀▀▄
█ ▄█▄▄▄█▄ █   ▄▀▀▀▀ ▄▀▀▀▄ █▄ ▄█ █▀▀▀▄ ▄▀▀▀▄
█  █   █  █    ▀▀▀▄ █▀▀▀█ █ ▀ █ █▀▀▀  █   █
█ ▀█▀▀▀█▀ █   ▀▀▀▀  ▀   ▀ ▀   ▀ ▀      ▀▀▀
 ▀▄▄▄▄▄▄▄▀    internet relay chat · forged in rust
```

# sampo

An IRC server and a terminal IRC client, written in Rust the way an IRC
daemon was written in 1993: one loop, one thread, no locks, and RFC 1459
open on the desk.

In the Kalevala, Ilmarinen the smith forged the Sampo, a mill that ground
out grain, salt and gold from nothing at all, and everyone wanted it. IRC
was born in Finland too, at the University of Oulu in August 1988. This is a
small Sampo for that small, durable thing. It grinds out conversation.

## Quick start

```sh
cargo build --release

./target/release/sampo serve        # in one terminal
./target/release/sampo connect      # in another
```

Type `/join #sampo`, then talk. Open a second `sampo connect` (or point any
IRC client at `localhost:6667`: irssi, WeeChat, HexChat, even `telnet`) and
it becomes a conversation. Ctrl-C stops either end cleanly.

## The server

```text
$ sampo serve
 ▄▀▀▀▀▀▀▀▄
█ ▄█▄▄▄█▄ █   ▄▀▀▀▀ ▄▀▀▀▄ █▄ ▄█ █▀▀▀▄ ▄▀▀▀▄
█  █   █  █    ▀▀▀▄ █▀▀▀█ █ ▀ █ █▀▀▀  █   █
█ ▀█▀▀▀█▀ █   ▀▀▀▀  ▀   ▀ ▀   ▀ ▀      ▀▀▀
 ▀▄▄▄▄▄▄▄▀    internet relay chat · forged in rust

2026-10-20 19:07:50 sampo.local sampo-0.1.0 starting
2026-10-20 19:07:50 listening on 0.0.0.0:6667
2026-10-20 19:07:51 #1 connected from 127.0.0.1:53908
2026-10-20 19:07:51 #1 registered as alice!~alice@127.0.0.1
2026-10-20 19:07:53 #2 connected from 127.0.0.1:53909
2026-10-20 19:07:53 #2 registered as bob!~bob@127.0.0.1
2026-10-20 19:07:54 #2 bob is now known as robert
2026-10-20 19:07:56 #2 robert!~bob@127.0.0.1 exited (Quit: off to the sauna)
2026-10-20 19:08:02 Server shutting down
2026-10-20 19:08:02 shut down cleanly
```

| Option              | Meaning                                                      |
| ------------------- | ------------------------------------------------------------ |
| `-l, --listen ADDR` | where to listen: `6667`, `127.0.0.1:6667`, `[::]:6667`; repeatable. Default `0.0.0.0:6667` |
| `-n, --name NAME`   | the server's name, which must contain a dot. Default `sampo.local` |
| `--network NAME`    | the network's name. Default `Sampo`                          |
| `--info TEXT`       | a line about the server, for WHOIS                           |
| `-m, --motd FILE`   | the message of the day. The default one carries the emblem   |
| `--password PASS`   | a password every client must give with PASS                  |
| `--oper NAME:PASS`  | an IRC operator, for OPER; repeatable                        |
| `--max-clients N`   | the most connections at once. Default 1024                   |

Ctrl-C, SIGTERM, closing the console window on Windows, or an operator's
`DIE` all shut the server down the same way. It closes the listening
sockets first, so the port is free again at once, then sends every client
`ERROR :Closing Link: ... (Server shutting down)` and gives the farewells a
moment to drain.

To keep it to your own machine, use `sampo serve -l 127.0.0.1:6667`.

## The client

The screen is ircII's. The conversation runs down from the top, the status
line sits under it in reverse video, and you type at the foot:

```text
19:07 *** alice (~alice@127.0.0.1) has joined channel #sampo
19:07 *** Mode change "+nt" on channel #sampo by sampo.local
19:07 *** Users on #sampo: @alice
19:07 *** bob (~bob@127.0.0.1) has joined channel #sampo
19:07 <bob> morning, alice
19:07 > morning, bob
19:07 * bob waves
19:07 *bob* psst, a secret
19:07 *** bob is now known as robert
19:07 *** robert has left channel #sampo (off to the sauna)
 19:08 @alice on #sampo(+nt) [Act: robert]                     sampo.local
[#sampo] _
```

Everything arrives in the one window. A message from a channel you are not
talking in is marked `<nick:#channel>`, and the status line keeps a list of
where there is news. mIRC colours and bold are shown, and every other
control character is dropped before it reaches your terminal.

```sh
sampo connect                              # localhost:6667, as your login name
sampo connect irc.example.net 6667 -n ilmari -j sampo,rust
```

| Option                | Meaning                                     |
| --------------------- | ------------------------------------------- |
| `-n, --nick NICK`     | your nickname. Default: your login name     |
| `-u, --user NAME`     | your username. Default: your nickname       |
| `-r, --realname TEXT` | your real name, as WHOIS shows it           |
| `-p, --password PASS` | the server's password, if it has one        |
| `-j, --join CHANNELS` | channels to join, separated by commas       |

### Commands

| Command                         | Does                                                    |
| ------------------------------- | ------------------------------------------------------- |
| `/join #channel [key]`          | join a channel, or switch to one you are already in     |
| `/part [#channel] [message]`    | leave a channel                                         |
| `/msg nick text`                | a private message; `/msg , text` answers whoever wrote last |
| `/query [nick]`                 | make someone the default target; alone, stop            |
| `/me action`                    | `* alice waves`                                         |
| `/notice target text`           | send a notice                                           |
| `/nick name`                    | change your nickname                                    |
| `/topic [#channel] [text]`      | show the topic, or set it                               |
| `/names`, `/who`, `/whois`, `/list` | ask who is where                                    |
| `/mode`, `/umode`               | show or change modes; `/mode +m` means the current channel |
| `/op`, `/deop`, `/voice`, `/devoice` | up to four nicks at once                           |
| `/kick nick [reason]`, `/invite nick` | for channel operators                             |
| `/away [message]`, `/back`      | come and go                                             |
| `/ctcp nick request`, `/ping nick` | CTCP, and the round-trip time to someone             |
| `/quote line`                   | send a raw line                                         |
| `/server [host[:port]]`         | connect, or connect again                               |
| `/clear`, `/help`, `/quit [message]` |                                                    |

As in ircII, a command the client does not know goes to the server as it
stands, so `/motd`, `/lusers`, `/version`, `/time`, `/info`, `/oper` and the
rest all just work. Start a line with `//` to say something that begins with
a slash.

### Keys

| Key                         | Does                                    |
| --------------------------- | --------------------------------------- |
| Enter                       | send                                    |
| Tab                         | complete a nick, channel or command; again for the next |
| Up, Down                    | earlier and later lines                 |
| PgUp, PgDn                  | scroll back and forth                   |
| Ctrl-X or Ctrl-N, Ctrl-P    | next and previous channel or query      |
| Alt-1 … Alt-9               | straight to a channel or query          |
| Ctrl-A, Ctrl-E, Ctrl-U, Ctrl-K, Ctrl-W | line editing, Emacs style    |
| Ctrl-L                      | redraw the screen                       |
| Ctrl-C, or Ctrl-D on an empty line | quit                             |

Set `NO_COLOR` to turn colour off.

## What it speaks

**Commands:** PASS, NICK, USER, QUIT, PING, PONG, PRIVMSG, NOTICE, JOIN,
PART, TOPIC, NAMES, LIST, INVITE, KICK, MODE, WHO, WHOIS, USERHOST, ISON,
AWAY, MOTD, LUSERS, VERSION, TIME, INFO, OPER, KILL, WALLOPS and DIE.

**User modes:** `+i` invisible, `+w` receives WALLOPS, `+o` IRC operator
(given by OPER only, though anyone may give it up).

**Channel modes:** `+o` operator and `+v` voice; `+b` bans, with masks
filled out the way ircd did (`alice` bans `alice!*@*`); `+i` invite-only,
`+k` key, `+l` limit, `+m` moderated, `+n` no messages from outside, `+t`
topic set by operators only, and `+p`/`+s` private and secret. New channels
are `+nt` and their creator is an operator. An invitation opens every lock
except the key.

**The welcome** is the one 1990s clients expect: 001 to 004, `RPL_ISUPPORT`
(005) with `CASEMAPPING=rfc1459` and the rest, the user count, and the
message of the day. Replies carry RFC 1459's own wording, misspellings and
all: *Erroneus nickname*, *Cant change mode for other users*.

**Names** are compared the Scandinavian way. RFC 1459 holds that `{}|` are
the lower case of `[]\`. On the Finnish terminals IRC was born on, those
bytes printed as `äöå` and `ÄÖÅ`, so `[ilmari]` and `{ILMARI}` are one
nick here.

**Old habits kept:** a fresh connection is greeted with `NOTICE AUTH`;
overlong nicks and lines are cut short rather than refused; an idle client
is sent a PING and dropped with `Ping timeout` if it says nothing; a client
that stops reading gets `Max SendQ exceeded`; a flood is slowed by ircd
2.8's fake lag, ten commands free and then one a second.

## Simplifications, on purpose

* **One server, no network.** There is no server-to-server linking: no
  SERVER, SQUIT, CONNECT or LINKS, no timestamps, no netsplits. Hop counts
  are always 0.
* **No DNS or ident.** A user's host is their IP address, and every
  username carries the `~` that says no ident server vouched for it.
* **Plain TCP, as in 1993.** No TLS, no IRCv3. CAP gets `421 Unknown
  command`, which modern clients take in their stride, and message tags
  sent anyway are skipped over. Put stunnel in front for encryption.
* **`#` channels only.** No `&` local channels; with one server, everything
  is local.
* **`+p` and `+s` behave alike.** Both hide a channel from outsiders, in
  LIST, NAMES, WHO and WHOIS alike; there is no `Prv` in LIST.
* **The 1990s mode set only:** no ban exceptions (`+e`), invite exceptions
  (`+I`), half-operators, or services.
* **Nicknames run to 16 characters**, not RFC 1459's 9, a concession to
  the present.
* **Text is UTF-8.** Input that is not valid UTF-8 is read as Latin-1,
  which is what 8-bit clients nearly always meant, and passed on as UTF-8.
  The old servers just relayed raw bytes.
* **WHO without a mask lists everyone you can see.** RFC 1459's rule of
  leaving out people who share a channel with you is skipped.
* **Missing:** WHOWAS, STATS, ADMIN, TRACE, USERS, SUMMON, REHASH, RESTART,
  and the `$server` and `#host` targets that operators once messaged.
* **No ircd.conf.** Configuration is command-line flags; operators are
  `--oper NAME:PASS`, matched without regard to host.
* **The client is ircII's default single window**, without /window, DCC or
  scripting.

## How it is built

```text
src/
├── main.rs, cli.rs        the binary: getopt-style arguments, one thread
├── banner.rs              the emblem, the MOTD, the INFO text
├── proto/                 RFC 1459, shared by both ends
│   ├── message.rs         parsing and writing messages
│   ├── numeric.rs         the numerics, named as in the RFC
│   ├── casemap.rs         Scandinavian case folding and wildcard masks
│   └── line.rs            cutting a byte stream into 512-byte lines
├── server/
│   ├── mod.rs             the Server: all state, and the one loop that runs it
│   ├── conn.rs            reader and writer tasks for each socket; fake lag
│   ├── client.rs          a connection and its user
│   ├── channel.rs         a channel, its members and modes
│   ├── config.rs          the configuration
│   ├── error.rs           startup errors, and the RFC's error replies as an enum
│   └── commands/          the command table (msgtab[], after parse.c), and
│                          user, message, channel, mode, info and oper commands
└── client/
    ├── mod.rs             the App: keyboard, network and clock in one select!
    ├── net.rs             dialling, reading and writing the server
    ├── session.rs         what the client knows: channels, members, modes
    ├── handlers.rs        what it makes of what the server says
    ├── commands.rs        what it makes of what you type
    ├── render.rs          ircII's formats, and the status line
    ├── screen.rs          layout, scrollback, drawing
    ├── editor.rs          the input line: history, completion
    ├── text.rs            styled text, mIRC colours, wrapping
    └── term.rs            raw mode in, everything restored on the way out
```

The original ircd was a single process around a single `select()` call, and
sampo keeps that shape. A `Server` owns every client, nick and channel. A
reader and a writer task for each socket do nothing but move lines between
the socket and the server's queues. Every NICK, JOIN and PRIVMSG is decided
in one place, one at a time, with no locks anywhere, and the whole program
runs on Tokio's single-threaded runtime.

```text
socket ─▶ reader ─┐                     ┌─▶ queue ─▶ writer ─▶ socket
socket ─▶ reader ─┼─▶ events ─▶ Server ─┼─▶ queue ─▶ writer ─▶ socket
listener ─────────┘                     └─▶ queue ─▶ writer ─▶ socket
```

A few choices follow from that shape. Handlers return
`Result<(), ErrorReply>`, so `?` sends the right numeric back. A
serialized line goes to every recipient as one shared `Arc<str>`. A client
found dead while being written to is reaped once the current event is done,
as ircd did with its dead sockets. Parameters are scrubbed of CR, LF and NUL
on the way out, so no text can smuggle in a line of its own.

Four dependencies, each earning its place: **tokio** for sockets, timers
and signals; **crossterm** for the terminal on Windows and Unix alike;
**chrono** for local time with daylight saving on every platform; and
**unicode-width** so that wide characters wrap where they should. The
command line, the error types and the protocol are written out by hand.

## Platforms

Windows, Linux, macOS and FreeBSD, on stable Rust 1.88 or later. On Unix the
server stops on SIGINT or SIGTERM; on Windows, on Ctrl-C, Ctrl-Break, or the
console closing. Listening on 0.0.0.0 may draw a question from your
firewall the first time.

## Tests

```sh
cargo test
```

Unit tests cover the parser, case mapping, framing, mode handling,
wrapping, the editor and the client's handling of server traffic. A VT100
replay checks the screen layout, and end-to-end tests run a real server on
a spare port and talk to it over real sockets. They cover the welcome,
channels, modes, bans, kicks, passwords, operators, abrupt disconnects, and
a shutdown that leaves the port free.

## Thanks

To Jarkko Oikarinen, who wrote IRC at the University of Oulu in 1988, and
to Darren Reed, who with him set it down as RFC 1459 in May 1993. To
tolsun.oulu.fi, the first server. To ircII, whose screen this client
borrows. And to everyone who ever kept a server up at three in the morning.
