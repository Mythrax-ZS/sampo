//! Registration, and the commands that concern only the sender.

use super::{Outcome, clip, now, valid_nick};
use crate::banner;
use crate::proto::numeric::*;
use crate::proto::{Folded, Message};
use crate::server::channel::{ALL_MODES, PARAM_MODES};
use crate::server::client::{ClientId, UserModes};
use crate::server::{
    AWAYLEN, CHANNELLEN, ErrorReply, KICKLEN, Line, MAX_CHANNELS, MAX_MODES, MAX_TARGETS, NICKLEN,
    REALLEN, Server, TOPICLEN, USERLEN,
};

impl Server {
    /// `PASS <password>`: must come before NICK and USER.
    pub(super) fn pass(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let client = self.client_mut(id);
        if client.registered {
            return Err(ErrorReply::AlreadyRegistered);
        }
        client.password = Some(msg.params[0].clone());
        Ok(())
    }

    /// `NICK <nickname>`: chooses a nickname, or changes it.
    pub(super) fn nick(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let wanted = msg.arg(0).unwrap_or_default();
        if wanted.is_empty() {
            return Err(ErrorReply::NoNicknameGiven);
        }
        // Too long a nickname is cut short, as ircd always did.
        let wanted = clip(wanted, NICKLEN).to_owned();
        if !valid_nick(&wanted) {
            return Err(ErrorReply::ErroneousNickname(wanted));
        }
        let key = Folded::new(&wanted);
        if self.nicks.get(&key).is_some_and(|&owner| owner != id) {
            return Err(ErrorReply::NicknameInUse(wanted));
        }
        let client = self.client(id);
        if client.nick.as_deref() == Some(wanted.as_str()) {
            return Ok(());
        }
        if client.registered {
            let change = Message::new("NICK").with_prefix(client.mask()).trailing(&wanted);
            let change = Line::from(change.to_line());
            self.send_line(id, &change);
            for peer in self.peers(id) {
                self.send_line(peer, &change);
            }
            log!("#{id} {} is now known as {wanted}", client.nick());
        }
        if let Some(old) = self.client_mut(id).nick.replace(wanted) {
            self.nicks.remove(&Folded::new(&old));
        }
        self.nicks.insert(key, id);
        self.try_register(id);
        Ok(())
    }

    /// `USER <username> <mode> <unused> <realname>`: says who is behind
    /// the connection. The mode is RFC 2812's bit mask: 8 asks for `+i`,
    /// 4 for `+w`.
    pub(super) fn user(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let client = self.client_mut(id);
        if client.registered || client.user.is_some() {
            return Err(ErrorReply::AlreadyRegistered);
        }
        // No ident server vouches for the username, and the tilde says so.
        let username: String = msg.params[0]
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || "-._".contains(*c))
            .take(USERLEN - 1)
            .collect();
        let username = if username.is_empty() { "user" } else { &username };
        client.user = Some(format!("~{username}"));
        if let Ok(bits) = msg.params[1].parse::<u32>() {
            client.modes.invisible = bits & 8 != 0;
            client.modes.wallops = bits & 4 != 0;
        }
        client.realname = clip(&msg.params[3], REALLEN).to_owned();
        self.try_register(id);
        Ok(())
    }

    /// `QUIT [<message>]`
    pub(super) fn quit(&mut self, id: ClientId, msg: &Message) -> Outcome {
        // The "Quit:" keeps users from passing off a farewell as a
        // "Ping timeout" or a kill.
        let reason = match msg.arg(0).filter(|text| !text.is_empty()) {
            Some(text) => format!("Quit: {}", clip(text, TOPICLEN)),
            None => String::from("Client Quit"),
        };
        self.exit_client(id, &reason);
        Ok(())
    }

    /// `PING <token>`: answered with the same token.
    pub(super) fn ping(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let Some(token) = msg.arg(0) else {
            return Err(ErrorReply::NoOrigin);
        };
        let name = &self.config.name;
        self.send(id, Message::new("PONG").with_prefix(name).param(name).trailing(token));
        Ok(())
    }

    /// `PONG <token>`: the dispatcher has already noted that the client is
    /// alive, which is all a PONG is for.
    pub(super) fn pong(&mut self, _: ClientId, _: &Message) -> Outcome {
        Ok(())
    }

    /// `AWAY [<message>]`: with a message, marks the sender away; without
    /// one, back again.
    pub(super) fn away(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let message = msg.arg(0).filter(|text| !text.is_empty());
        self.client_mut(id).away = message.map(|text| clip(text, AWAYLEN).to_owned());
        let reply = match message {
            Some(_) => self.reply(id, RPL_NOWAWAY).trailing("You have been marked as being away"),
            None => self.reply(id, RPL_UNAWAY).trailing("You are no longer marked as being away"),
        };
        self.send(id, reply);
        Ok(())
    }

    /// Completes registration once NICK and USER have both arrived.
    fn try_register(&mut self, id: ClientId) {
        let client = self.client(id);
        if client.registered || client.nick.is_none() || client.user.is_none() {
            return;
        }
        if let Some(password) = &self.config.password
            && client.password.as_ref() != Some(password)
        {
            self.send_error(id, ErrorReply::PasswdMismatch);
            self.exit_client(id, "Bad Password");
            return;
        }
        let client = self.client_mut(id);
        client.registered = true;
        client.signon = now();
        self.welcome(id);
    }

    /// The greeting every client gets on registering: 001 to 005, then the
    /// user count and the message of the day.
    fn welcome(&mut self, id: ClientId) {
        let client = self.client(id);
        let (mask, modes) = (client.mask(), client.modes);
        let name = &self.config.name;
        let version = banner::server_version();

        let text =
            format!("Welcome to the {} Internet Relay Chat Network {mask}", self.config.network);
        self.send(id, self.reply(id, RPL_WELCOME).trailing(text));
        let text = format!("Your host is {name}, running version {version}");
        self.send(id, self.reply(id, RPL_YOURHOST).trailing(text));
        let text = format!("This server was created {}", self.created);
        self.send(id, self.reply(id, RPL_CREATED).trailing(text));
        self.send(
            id,
            self.reply(id, RPL_MYINFO).params([
                name.as_str(),
                version.as_str(),
                UserModes::ALL,
                ALL_MODES,
                PARAM_MODES,
            ]),
        );
        self.send(
            id,
            self.reply(id, RPL_ISUPPORT)
                .params(self.isupport())
                .trailing("are supported by this server"),
        );
        self.send_lusers(id);
        self.send_motd(id);
        if !modes.is_empty() {
            let nick = self.client(id).nick();
            let change =
                Message::new("MODE").with_prefix(nick).param(nick).trailing(modes.to_string());
            self.send(id, change);
        }
        log!("#{id} registered as {mask}");
    }

    /// What this server supports, as `RPL_ISUPPORT` tokens.
    fn isupport(&self) -> Vec<String> {
        vec![
            "CHANTYPES=#".into(),
            "PREFIX=(ov)@+".into(),
            "CHANMODES=b,k,l,imnpst".into(),
            format!("MODES={MAX_MODES}"),
            "CASEMAPPING=rfc1459".into(),
            format!("NICKLEN={NICKLEN}"),
            format!("CHANNELLEN={CHANNELLEN}"),
            format!("TOPICLEN={TOPICLEN}"),
            format!("KICKLEN={KICKLEN}"),
            format!("AWAYLEN={AWAYLEN}"),
            format!("CHANLIMIT=#:{MAX_CHANNELS}"),
            format!("MAXTARGETS={MAX_TARGETS}"),
            format!("NETWORK={}", self.config.network),
        ]
    }
}
