//! The server, end to end: real sockets, and clients that speak raw IRC,
//! like someone typing into telnet.

use std::net::SocketAddr;
use std::time::Duration;

use sampo::proto::{LineReader, Message};
use sampo::server::{Config, Oper, Server};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// How long to wait for anything before calling it a failure.
const PATIENCE: Duration = Duration::from_secs(5);

/// A server on a port of its own, running until stopped.
struct TestServer {
    addr: SocketAddr,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn start() -> TestServer {
        TestServer::with(Config::default()).await
    }

    async fn with(config: Config) -> TestServer {
        let config = Config { listen: vec!["127.0.0.1:0".parse().unwrap()], ..config };
        let server = Server::bind(config).await.expect("the server binds");
        let addr = server.local_addrs()[0];
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(server.run(async {
            let _ = stopped.await;
        }));
        TestServer { addr, stop, task }
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        timeout(Duration::from_secs(10), self.task)
            .await
            .expect("the server stops in time")
            .expect("the server stops cleanly");
    }
}

/// A client speaking raw IRC.
struct Telnet {
    lines: LineReader<OwnedReadHalf>,
    out: OwnedWriteHalf,
}

impl Telnet {
    async fn connect(addr: SocketAddr) -> Telnet {
        let stream = TcpStream::connect(addr).await.expect("the server answers");
        let (reader, out) = stream.into_split();
        Telnet { lines: LineReader::new(reader), out }
    }

    /// Connects and registers, reading up to the end of the welcome.
    async fn login(addr: SocketAddr, nick: &str) -> Telnet {
        let mut client = Telnet::connect(addr).await;
        client.send(&format!("NICK {nick}")).await;
        client.send(&format!("USER {nick} 0 * :{nick} the tester")).await;
        client.expect(" 376 ").await;
        client
    }

    async fn send(&mut self, line: &str) {
        self.out
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .expect("the server is listening");
    }

    /// Reads lines until one contains `needle`, and returns it.
    async fn expect(&mut self, needle: &str) -> String {
        loop {
            let line = timeout(PATIENCE, self.lines.next_line())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}"))
                .expect("the connection holds")
                .unwrap_or_else(|| panic!("the connection closed while waiting for {needle:?}"));
            if line.contains(needle) {
                return line;
            }
        }
    }

    /// Reads lines until the server hangs up, and returns them.
    async fn until_closed(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            match timeout(PATIENCE, self.lines.next_line()).await {
                Ok(Ok(Some(line))) => lines.push(line),
                Ok(Ok(None) | Err(_)) => return lines,
                Err(_) => panic!("the server never hung up; heard {lines:#?}"),
            }
        }
    }
}

#[tokio::test]
async fn registration_brings_the_classic_welcome() {
    let server = TestServer::start().await;
    let mut alice = Telnet::connect(server.addr).await;
    alice.expect("NOTICE AUTH :*** Hello").await;
    alice.send("NICK alice").await;
    alice.send("USER alice 0 * :Alice Liddell").await;
    let welcome = alice.expect(" 001 ").await;
    assert_eq!(
        welcome,
        ":sampo.local 001 alice :Welcome to the Sampo Internet Relay Chat Network alice!~alice@127.0.0.1"
    );
    let isupport = alice.expect(" 005 ").await;
    assert!(isupport.contains("CASEMAPPING=rfc1459") && isupport.contains("PREFIX=(ov)@+"));
    alice.expect("forged in rust").await;
    alice.expect(" 376 ").await;
    server.stop().await;
}

#[tokio::test]
async fn two_clients_talk_in_a_channel() {
    let server = TestServer::start().await;
    let mut alice = Telnet::login(server.addr, "alice").await;
    let mut bob = Telnet::login(server.addr, "bob").await;

    alice.send("JOIN #sampo").await;
    alice.expect(":alice!~alice@127.0.0.1 JOIN #sampo").await;
    alice.expect(":sampo.local MODE #sampo +nt").await;
    assert_eq!(alice.expect(" 353 ").await, ":sampo.local 353 alice = #sampo :@alice");

    bob.send("JOIN #SAMPO").await;
    alice.expect(":bob!~bob@127.0.0.1 JOIN #sampo").await;
    assert_eq!(bob.expect(" 353 ").await, ":sampo.local 353 bob = #sampo :@alice bob");

    bob.send("PRIVMSG #sampo :hello, world").await;
    alice.expect(":bob!~bob@127.0.0.1 PRIVMSG #sampo :hello, world").await;

    alice.send("TOPIC #sampo :the mill grinds on").await;
    bob.expect(":alice!~alice@127.0.0.1 TOPIC #sampo :the mill grinds on").await;

    alice.send("MODE #sampo +o bob").await;
    bob.expect(":alice!~alice@127.0.0.1 MODE #sampo +o bob").await;

    bob.send("PART #sampo :off to the sauna").await;
    alice.expect(":bob!~bob@127.0.0.1 PART #sampo :off to the sauna").await;

    // Alice never heard her own words echoed back to her.
    alice.send("NAMES #sampo").await;
    assert_eq!(alice.expect(" 353 ").await, ":sampo.local 353 alice = #sampo :@alice");
    server.stop().await;
}

#[tokio::test]
async fn nicknames_collide_the_scandinavian_way() {
    let server = TestServer::start().await;
    let _alice = Telnet::login(server.addr, "[alice]").await;
    let mut other = Telnet::connect(server.addr).await;
    other.send("NICK {ALICE}").await;
    other.expect(":sampo.local 433 * {ALICE} :Nickname is already in use").await;
    other.send("NICK 0day").await;
    other.expect(" 432 * 0day :Erroneus nickname").await;
    server.stop().await;
}

#[tokio::test]
async fn abrupt_disconnects_are_noticed() {
    let server = TestServer::start().await;
    let mut alice = Telnet::login(server.addr, "alice").await;
    let mut bob = Telnet::login(server.addr, "bob").await;
    alice.send("JOIN #sampo").await;
    alice.expect(" 366 ").await;
    bob.send("JOIN #sampo").await;
    bob.expect(" 366 ").await;

    drop(bob);
    let quit = alice.expect(":bob!~bob@127.0.0.1 QUIT").await;
    let reason = Message::parse(&quit).unwrap().params[0].clone();
    assert!(
        reason == "Remote host closed the connection" || reason.starts_with("Read error"),
        "{reason}"
    );
    server.stop().await;
}

#[tokio::test]
async fn private_messages_tell_of_absence() {
    let server = TestServer::start().await;
    let mut alice = Telnet::login(server.addr, "alice").await;
    let mut bob = Telnet::login(server.addr, "bob").await;
    alice.send("AWAY :fishing").await;
    alice.expect(" 306 alice :You have been marked as being away").await;
    bob.send("PRIVMSG alice :are you there?").await;
    alice.expect(":bob!~bob@127.0.0.1 PRIVMSG alice :are you there?").await;
    bob.expect(":sampo.local 301 bob alice :fishing").await;
    bob.send("PRIVMSG nobody :hello?").await;
    bob.expect(" 401 bob nobody :No such nick/channel").await;
    server.stop().await;
}

#[tokio::test]
async fn channel_modes_keep_people_out() {
    let server = TestServer::start().await;
    let mut alice = Telnet::login(server.addr, "alice").await;
    let mut bob = Telnet::login(server.addr, "bob").await;
    alice.send("JOIN #hall").await;
    alice.send("MODE #hall +ik door").await;
    alice.expect("MODE #hall +ik door").await;

    bob.send("JOIN #hall").await;
    bob.expect(" 473 bob #hall :Cannot join channel (+i)").await;
    alice.send("INVITE bob #hall").await;
    alice.expect(" 341 alice bob #hall").await;
    bob.expect(":alice!~alice@127.0.0.1 INVITE bob #hall").await;
    // An invitation opens every lock but the key.
    bob.send("JOIN #hall").await;
    bob.expect(" 475 bob #hall :Cannot join channel (+k)").await;
    bob.send("JOIN #hall door").await;
    alice.expect(":bob!~bob@127.0.0.1 JOIN #hall").await;

    alice.send("MODE #hall +m").await;
    bob.expect("MODE #hall +m").await;
    bob.send("PRIVMSG #hall :may I speak?").await;
    bob.expect(" 404 bob #hall :Cannot send to channel").await;
    bob.send("MODE #hall -m").await;
    bob.expect(" 482 bob #hall :You're not channel operator").await;
    server.stop().await;
}

#[tokio::test]
async fn bans_and_kicks() {
    let server = TestServer::start().await;
    let mut alice = Telnet::login(server.addr, "alice").await;
    let mut bob = Telnet::login(server.addr, "bob").await;
    alice.send("JOIN #sampo").await;
    alice.expect(" 366 ").await;
    bob.send("JOIN #sampo").await;
    bob.expect(" 366 ").await;

    alice.send("MODE #sampo +b bob").await;
    bob.expect(":alice!~alice@127.0.0.1 MODE #sampo +b bob!*@*").await;
    alice.send("KICK #sampo bob :enough").await;
    bob.expect(":alice!~alice@127.0.0.1 KICK #sampo bob :enough").await;
    bob.send("JOIN #sampo").await;
    bob.expect(" 474 bob #sampo :Cannot join channel (+b)").await;
    server.stop().await;
}

#[tokio::test]
async fn commands_wait_for_registration() {
    let server = TestServer::start().await;
    let mut early = Telnet::connect(server.addr).await;
    early.send("JOIN #sampo").await;
    early.expect(" 451 * :You have not registered").await;
    early.send("NICK early").await;
    early.send("USER early 0 * :Early Bird").await;
    early.expect(" 001 ").await;
    early.send("FROBNICATE").await;
    early.expect(" 421 early FROBNICATE :Unknown command").await;
    early.send("USER again 0 * :Again").await;
    early.expect(" 462 early :You may not reregister").await;
    server.stop().await;
}

#[tokio::test]
async fn a_password_keeps_strangers_out() {
    let server =
        TestServer::with(Config { password: Some("sesame".into()), ..Config::default() }).await;
    let mut stranger = Telnet::connect(server.addr).await;
    stranger.send("NICK stranger").await;
    stranger.send("USER stranger 0 * :Stranger").await;
    let last_words = stranger.until_closed().await;
    assert!(last_words.iter().any(|l| l.contains(" 464 ")), "{last_words:#?}");
    assert!(
        last_words.iter().any(|l| l.starts_with("ERROR :Closing Link: 127.0.0.1 (Bad Password)"))
    );

    let mut friend = Telnet::connect(server.addr).await;
    friend.send("PASS sesame").await;
    friend.send("NICK friend").await;
    friend.send("USER friend 0 * :Friend").await;
    friend.expect(" 001 ").await;
    server.stop().await;
}

#[tokio::test]
async fn operators_can_remove_users() {
    let server = TestServer::with(Config {
        opers: vec![Oper { name: "root".into(), password: "hunter2".into() }],
        ..Config::default()
    })
    .await;
    let mut alice = Telnet::login(server.addr, "alice").await;
    let mut bob = Telnet::login(server.addr, "bob").await;
    bob.send("KILL alice :nope").await;
    bob.expect(" 481 bob :Permission Denied- You're not an IRC operator").await;
    alice.send("OPER root wrong").await;
    alice.expect(" 464 alice :Password incorrect").await;
    alice.send("OPER root hunter2").await;
    alice.expect(" 381 alice :You are now an IRC operator").await;
    alice.send("KILL bob :be gone").await;
    let last_words = bob.until_closed().await;
    assert!(last_words.iter().any(|l| l.contains("KILL bob :alice (be gone)")), "{last_words:#?}");
    assert!(last_words.iter().any(|l| l.contains("Killed (alice (be gone))")));
    server.stop().await;
}

#[tokio::test]
async fn shutting_down_says_goodbye_and_frees_the_port() {
    let server = TestServer::start().await;
    let addr = server.addr;
    let mut alice = Telnet::login(addr, "alice").await;
    server.stop().await;
    let last_words = alice.until_closed().await;
    assert!(
        last_words.iter().any(|l| l == "ERROR :Closing Link: 127.0.0.1 (Server shutting down)"),
        "{last_words:#?}"
    );
    TcpListener::bind(addr).await.expect("the port is free again");
}
