//! Questions: who is here, who is that, and what is this place.

use super::Outcome;
use crate::banner;
use crate::proto::numeric::*;
use crate::proto::{Folded, Message, casemap};
use crate::server::channel::Member;
use crate::server::client::{Client, ClientId};
use crate::server::{ErrorReply, MAX_TARGETS, Server};

impl Server {
    /// `WHO [<mask> [o]]`: lists users in a channel, or users whose nick,
    /// username, host, server or real name matches a mask. With `o`, only
    /// IRC operators.
    pub(super) fn who(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let mask = match msg.arg(0) {
            None | Some("" | "0") => "*",
            Some(mask) => mask,
        };
        let opers_only = msg.arg(1).is_some_and(|flags| flags.contains('o'));
        let wanted = |client: &Client| !opers_only || client.modes.operator;

        if mask.starts_with('#') {
            if let Some(channel) = self.channels.get(&Folded::new(mask)) {
                let inside = channel.is_member(id);
                if inside || !channel.is_hidden() {
                    for (member, standing) in &channel.members {
                        let Some(client) = self.clients.get(member) else {
                            continue;
                        };
                        if (inside || !client.modes.invisible) && wanted(client) {
                            self.send_who_reply(id, &channel.name, client, Some(*standing));
                        }
                    }
                }
            }
        } else {
            for client in self.clients.values().filter(|c| c.registered) {
                let matches = [
                    client.nick(),
                    client.user(),
                    &client.host,
                    &client.realname,
                    &self.config.name,
                ]
                .iter()
                .any(|field| casemap::wildcard(mask, field));
                if !matches || !wanted(client) || !self.can_see(id, client) {
                    continue;
                }
                // Name a channel they are in that the asker can see, if any.
                let channel = client
                    .channels
                    .iter()
                    .filter_map(|key| self.channels.get(key))
                    .find(|channel| channel.is_member(id) || !channel.is_hidden());
                match channel {
                    Some(channel) => {
                        self.send_who_reply(
                            id,
                            &channel.name,
                            client,
                            channel.members.get(&client.id).copied(),
                        );
                    }
                    None => self.send_who_reply(id, "*", client, None),
                }
            }
        }
        self.send(id, self.reply(id, RPL_ENDOFWHO).param(mask).trailing("End of /WHO list."));
        Ok(())
    }

    fn send_who_reply(
        &self,
        id: ClientId,
        channel: &str,
        client: &Client,
        standing: Option<Member>,
    ) {
        let mut flags = String::from(if client.away.is_some() { "G" } else { "H" });
        if client.modes.operator {
            flags.push('*');
        }
        flags.push_str(standing.map_or("", Member::prefix));
        let fields =
            [channel, client.user(), &client.host, &self.config.name, client.nick(), &flags];
        // A hop count of zero: everyone here is on this very server.
        let reply = self.reply(id, RPL_WHOREPLY).params(fields);
        self.send(id, reply.trailing(format!("0 {}", client.realname)));
    }

    /// `WHOIS [<server>] <nick>{,<nick>}`
    pub(super) fn whois(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let Some(nicks) = msg.params.last().filter(|n| !n.is_empty()) else {
            return Err(ErrorReply::NoNicknameGiven);
        };
        for nick in nicks.split(',').filter(|n| !n.is_empty()).take(MAX_TARGETS) {
            match self.find_nick(nick) {
                Some(client) => self.send_whois(id, client),
                None => self.send_error(id, ErrorReply::NoSuchNick(nick.to_owned())),
            }
        }
        self.send(id, self.reply(id, RPL_ENDOFWHOIS).param(nicks).trailing("End of /WHOIS list."));
        Ok(())
    }

    fn send_whois(&self, id: ClientId, client: &Client) {
        let nick = client.nick();
        let reply = self.reply(id, RPL_WHOISUSER).params([nick, client.user(), &client.host, "*"]);
        self.send(id, reply.trailing(&client.realname));

        let channels: Vec<String> = client
            .channels
            .iter()
            .filter_map(|key| self.channels.get(key))
            .filter(|channel| channel.is_member(id) || !channel.is_hidden())
            .map(|channel| {
                let standing = channel.members.get(&client.id).copied().unwrap_or_default();
                format!("{}{}", standing.prefix(), channel.name)
            })
            .collect();
        // Twenty channels of fifty bytes will not fit on one line.
        for chunk in channels.chunks(8) {
            self.send(id, self.reply(id, RPL_WHOISCHANNELS).param(nick).trailing(chunk.join(" ")));
        }

        let reply = self.reply(id, RPL_WHOISSERVER).param(nick).param(&self.config.name);
        self.send(id, reply.trailing(&self.config.info));
        if let Some(away) = &client.away {
            self.send(id, self.reply(id, RPL_AWAY).param(nick).trailing(away));
        }
        if client.modes.operator {
            self.send(
                id,
                self.reply(id, RPL_WHOISOPERATOR).param(nick).trailing("is an IRC operator"),
            );
        }
        let idle = client.last_spoke.elapsed().as_secs().to_string();
        let signon = client.signon.to_string();
        let reply = self.reply(id, RPL_WHOISIDLE).params([nick, &idle, &signon]);
        self.send(id, reply.trailing("seconds idle, signon time"));
    }

    /// `USERHOST <nick>{ <nick>}`: up to five at once.
    pub(super) fn userhost(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let replies: Vec<String> = msg
            .params
            .iter()
            .flat_map(|param| param.split(' '))
            .take(5)
            .filter_map(|nick| self.find_nick(nick))
            .map(|client| {
                let oper = if client.modes.operator { "*" } else { "" };
                let here = if client.away.is_some() { '-' } else { '+' };
                format!("{}{oper}={here}{}@{}", client.nick(), client.user(), client.host)
            })
            .collect();
        self.send(id, self.reply(id, RPL_USERHOST).trailing(replies.join(" ")));
        Ok(())
    }

    /// `ISON <nick>{ <nick>}`: which of these are online? The notify list
    /// of every 1990s client was built on this.
    pub(super) fn ison(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let online: Vec<&str> = msg
            .params
            .iter()
            .flat_map(|param| param.split(' '))
            .filter_map(|nick| self.find_nick(nick))
            .map(Client::nick)
            .collect();
        self.send(id, self.reply(id, RPL_ISON).trailing(online.join(" ")));
        Ok(())
    }

    /// `MOTD`
    pub(super) fn motd(&mut self, id: ClientId, _: &Message) -> Outcome {
        self.send_motd(id);
        Ok(())
    }

    pub(super) fn send_motd(&self, id: ClientId) {
        let lines = match &self.config.motd {
            Some(lines) => lines.clone(),
            None => banner::motd(&self.config.name),
        };
        if lines.is_empty() {
            return self.send_error(id, ErrorReply::NoMotd);
        }
        let start = format!("- {} Message of the day - ", self.config.name);
        self.send(id, self.reply(id, RPL_MOTDSTART).trailing(start));
        for line in lines {
            self.send(id, self.reply(id, RPL_MOTD).trailing(format!("- {line}")));
        }
        self.send(id, self.reply(id, RPL_ENDOFMOTD).trailing("End of /MOTD command."));
    }

    /// `LUSERS`
    pub(super) fn lusers(&mut self, id: ClientId, _: &Message) -> Outcome {
        self.send_lusers(id);
        Ok(())
    }

    pub(super) fn send_lusers(&self, id: ClientId) {
        let (mut visible, mut invisible, mut opers, mut unknown) = (0, 0, 0, 0);
        for client in self.clients.values() {
            match client {
                c if !c.registered => unknown += 1,
                c if c.modes.invisible => invisible += 1,
                _ => visible += 1,
            }
            if client.modes.operator {
                opers += 1;
            }
        }
        let text = format!("There are {visible} users and {invisible} invisible on 1 servers");
        self.send(id, self.reply(id, RPL_LUSERCLIENT).trailing(text));
        if opers > 0 {
            self.send(
                id,
                self.reply(id, RPL_LUSEROP).param(opers.to_string()).trailing("operator(s) online"),
            );
        }
        if unknown > 0 {
            let reply = self.reply(id, RPL_LUSERUNKNOWN).param(unknown.to_string());
            self.send(id, reply.trailing("unknown connection(s)"));
        }
        if !self.channels.is_empty() {
            let reply = self.reply(id, RPL_LUSERCHANNELS).param(self.channels.len().to_string());
            self.send(id, reply.trailing("channels formed"));
        }
        let text = format!("I have {} clients and 0 servers", visible + invisible);
        self.send(id, self.reply(id, RPL_LUSERME).trailing(text));
    }

    /// `VERSION`
    pub(super) fn version(&mut self, id: ClientId, _: &Message) -> Outcome {
        // `<version>.<debuglevel>`; there is no debug level.
        let version = format!("{}.", banner::server_version());
        let reply = self.reply(id, RPL_VERSION).param(version).param(&self.config.name);
        self.send(id, reply.trailing("internet relay chat, forged in rust"));
        Ok(())
    }

    /// `TIME`: the server's local time.
    pub(super) fn time(&mut self, id: ClientId, _: &Message) -> Outcome {
        let now = chrono::Local::now().format("%A %B %-d %Y -- %H:%M:%S %:z").to_string();
        self.send(id, self.reply(id, RPL_TIME).param(&self.config.name).trailing(now));
        Ok(())
    }

    /// `INFO`: where all this came from.
    pub(super) fn info(&mut self, id: ClientId, _: &Message) -> Outcome {
        for line in banner::INFO {
            self.send(id, self.reply(id, RPL_INFO).trailing(*line));
        }
        self.send(id, self.reply(id, RPL_ENDOFINFO).trailing("End of /INFO list."));
        Ok(())
    }
}
