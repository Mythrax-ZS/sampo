//! Channel commands: joining and leaving, topics, names, invitations and
//! kicks.

use super::{Outcome, clip, now, valid_channel};
use crate::proto::numeric::*;
use crate::proto::{Folded, Message};
use crate::server::channel::{Channel, Member, Topic};
use crate::server::client::ClientId;
use crate::server::{ErrorReply, KICKLEN, Line, MAX_CHANNELS, Server, TOPICLEN};

impl Server {
    /// `JOIN <channel>{,<channel>} [<key>{,<key>}]`, or `JOIN 0` to leave
    /// every channel at once.
    pub(super) fn join(&mut self, id: ClientId, msg: &Message) -> Outcome {
        if msg.params[0] == "0" {
            let channels: Vec<_> = self.client(id).channels.iter().cloned().collect();
            for key in channels {
                self.part_one(id, &key, None);
            }
            return Ok(());
        }
        let keys: Vec<&str> = msg.arg(1).map(|k| k.split(',').collect()).unwrap_or_default();
        for (i, name) in msg.params[0].split(',').enumerate() {
            if name.is_empty() {
                continue;
            }
            if let Err(error) = self.join_one(id, name, keys.get(i).copied()) {
                self.send_error(id, error);
            }
        }
        Ok(())
    }

    fn join_one(&mut self, id: ClientId, name: &str, key: Option<&str>) -> Outcome {
        if !valid_channel(name) {
            return Err(ErrorReply::NoSuchChannel(name.to_owned()));
        }
        let folded = Folded::new(name);
        let client = self.client(id);
        if client.channels.contains(&folded) {
            return Ok(());
        }
        if client.channels.len() >= MAX_CHANNELS {
            return Err(ErrorReply::TooManyChannels(name.to_owned()));
        }
        let mask = client.mask();

        let created = !self.channels.contains_key(&folded);
        let channel =
            self.channels.entry(folded.clone()).or_insert_with(|| Channel::new(name, now()));
        if !created {
            // An invitation opens every lock but the key.
            let invited = channel.invites.contains(&id);
            if !invited && channel.is_banned(&mask) {
                return Err(ErrorReply::BannedFromChan(channel.name.clone()));
            }
            if !invited && channel.modes.invite_only {
                return Err(ErrorReply::InviteOnlyChan(channel.name.clone()));
            }
            if !invited && channel.modes.limit.is_some_and(|limit| channel.members.len() >= limit) {
                return Err(ErrorReply::ChannelIsFull(channel.name.clone()));
            }
            if channel.modes.key.as_deref().is_some_and(|k| key != Some(k)) {
                return Err(ErrorReply::BadChannelKey(channel.name.clone()));
            }
            channel.invites.remove(&id);
        }
        channel.members.insert(id, Member { op: created, voice: false });
        self.client_mut(id).channels.insert(folded.clone());

        let Some(channel) = self.channels.get(&folded) else {
            return Ok(());
        };
        let join = Message::new("JOIN").with_prefix(mask).param(&channel.name);
        self.send_channel(channel, &Line::from(join.to_line()), None);
        if created {
            let (modes, _) = channel.modes.describe(false);
            let mode = Message::new("MODE")
                .with_prefix(&self.config.name)
                .param(&channel.name)
                .param(modes);
            self.send(id, mode);
        }
        self.send_topic(id, channel, false);
        self.send_names(id, channel, true);
        Ok(())
    }

    /// `PART <channel>{,<channel>} [<message>]`
    pub(super) fn part(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let reason = msg.arg(1).filter(|text| !text.is_empty());
        for name in msg.params[0].split(',').filter(|n| !n.is_empty()) {
            let key = Folded::new(name);
            match self.channels.get(&key) {
                None => self.send_error(id, ErrorReply::NoSuchChannel(name.to_owned())),
                Some(channel) if !channel.is_member(id) => {
                    self.send_error(id, ErrorReply::NotOnChannel(channel.name.clone()));
                }
                Some(_) => self.part_one(id, &key, reason),
            }
        }
        Ok(())
    }

    /// Announces a member's departure, and lets them go.
    fn part_one(&mut self, id: ClientId, key: &Folded, reason: Option<&str>) {
        let Some(channel) = self.channels.get(key) else {
            return;
        };
        let mut part =
            Message::new("PART").with_prefix(self.client(id).mask()).param(&channel.name);
        if let Some(reason) = reason {
            part = part.trailing(clip(reason, KICKLEN));
        }
        self.send_channel(channel, &Line::from(part.to_line()), None);
        self.remove_member(key, id);
    }

    /// Takes a client out of a channel, and the channel away when it empties.
    fn remove_member(&mut self, key: &Folded, id: ClientId) {
        if let Some(channel) = self.channels.get_mut(key) {
            channel.members.remove(&id);
            if channel.members.is_empty() {
                self.channels.remove(key);
            }
        }
        if let Some(client) = self.clients.get_mut(&id) {
            client.channels.remove(key);
        }
    }

    /// `TOPIC <channel> [<topic>]`: shows the topic, or sets it. An empty
    /// topic clears it.
    pub(super) fn topic(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let name = &msg.params[0];
        let key = Folded::new(name);
        let channel =
            self.channels.get(&key).ok_or_else(|| ErrorReply::NoSuchChannel(name.clone()))?;
        let member = channel.members.get(&id).copied();

        let Some(text) = msg.arg(1) else {
            if member.is_none() && channel.is_hidden() {
                return Err(ErrorReply::NotOnChannel(channel.name.clone()));
            }
            self.send_topic(id, channel, true);
            return Ok(());
        };
        let Some(member) = member else {
            return Err(ErrorReply::NotOnChannel(channel.name.clone()));
        };
        if channel.modes.topic_lock && !member.op {
            return Err(ErrorReply::ChanOpPrivsNeeded(channel.name.clone()));
        }

        let client = self.client(id);
        let (mask, nick) = (client.mask(), client.nick().to_owned());
        let text = clip(text, TOPICLEN);
        let change = Message::new("TOPIC").with_prefix(mask).param(&channel.name).trailing(text);
        self.send_channel(channel, &Line::from(change.to_line()), None);
        let topic = (!text.is_empty()).then(|| Topic {
            text: text.to_owned(),
            set_by: nick,
            set_at: now(),
        });
        if let Some(channel) = self.channels.get_mut(&key) {
            channel.topic = topic;
        }
        Ok(())
    }

    /// Sends a channel's topic, and who set it. With `or_none`, says so
    /// when there is no topic, rather than staying quiet.
    pub(super) fn send_topic(&self, id: ClientId, channel: &Channel, or_none: bool) {
        match &channel.topic {
            Some(topic) => {
                self.send(id, self.reply(id, RPL_TOPIC).param(&channel.name).trailing(&topic.text));
                let set_at = topic.set_at.to_string();
                self.send(
                    id,
                    self.reply(id, RPL_TOPICWHOTIME).params([
                        channel.name.as_str(),
                        topic.set_by.as_str(),
                        set_at.as_str(),
                    ]),
                );
            }
            None if or_none => {
                self.send(
                    id,
                    self.reply(id, RPL_NOTOPIC).param(&channel.name).trailing("No topic is set"),
                );
            }
            None => {}
        }
    }

    /// `NAMES [<channel>{,<channel>}]`
    pub(super) fn names(&mut self, id: ClientId, msg: &Message) -> Outcome {
        match msg.arg(0).filter(|list| !list.is_empty()) {
            Some(list) => {
                for name in list.split(',').filter(|n| !n.is_empty()) {
                    match self.channels.get(&Folded::new(name)) {
                        Some(channel) if channel.is_member(id) || !channel.is_hidden() => {
                            self.send_names(id, channel, true);
                        }
                        _ => self.send_end_of_names(id, name),
                    }
                }
            }
            None => {
                for channel in self.channels.values() {
                    if channel.is_member(id) || !channel.is_hidden() {
                        self.send_names(id, channel, false);
                    }
                }
                self.send_end_of_names(id, "*");
            }
        }
        Ok(())
    }

    /// Sends the members of a channel, as many to a line as fit. Members
    /// who are `+i` are left out unless the asker is in the channel too.
    pub(super) fn send_names(&self, id: ClientId, channel: &Channel, end: bool) {
        let inside = channel.is_member(id);
        let names = channel.members.iter().filter_map(|(member, standing)| {
            let client = self.clients.get(member)?;
            (inside || !client.modes.invisible)
                .then(|| format!("{}{}", standing.prefix(), client.nick()))
        });
        let head = self.reply(id, RPL_NAMREPLY).param(channel.kind()).param(&channel.name);
        // The room left on a line once the head and the colon are written.
        let room = 510usize.saturating_sub(head.to_string().len() + 2).max(64);
        let mut line = String::new();
        for name in names {
            if !line.is_empty() && line.len() + 1 + name.len() > room {
                self.send(id, head.clone().trailing(std::mem::take(&mut line)));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&name);
        }
        if !line.is_empty() {
            self.send(id, head.trailing(line));
        }
        if end {
            self.send_end_of_names(id, &channel.name);
        }
    }

    fn send_end_of_names(&self, id: ClientId, name: &str) {
        self.send(id, self.reply(id, RPL_ENDOFNAMES).param(name).trailing("End of /NAMES list."));
    }

    /// `LIST [<channel>{,<channel>}]`
    pub(super) fn list(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let wanted: Option<Vec<Folded>> = msg
            .arg(0)
            .filter(|list| !list.is_empty())
            .map(|list| list.split(',').map(Folded::new).collect());
        self.send(id, self.reply(id, RPL_LISTSTART).param("Channel").trailing("Users  Name"));
        for (key, channel) in &self.channels {
            if wanted.as_ref().is_some_and(|w| !w.contains(key)) {
                continue;
            }
            if channel.is_hidden() && !channel.is_member(id) {
                continue;
            }
            let topic = channel.topic.as_ref().map_or("", |t| t.text.as_str());
            let users = channel.members.len().to_string();
            self.send(
                id,
                self.reply(id, RPL_LIST).param(&channel.name).param(users).trailing(topic),
            );
        }
        self.send(id, self.reply(id, RPL_LISTEND).trailing("End of /LIST"));
        Ok(())
    }

    /// `INVITE <nickname> <channel>`
    pub(super) fn invite(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let (nick, name) = (&msg.params[0], &msg.params[1]);
        let target = self.find_nick(nick).ok_or_else(|| ErrorReply::NoSuchNick(nick.clone()))?;
        let (target_id, target_nick) = (target.id, target.nick().to_owned());
        let away = target.away.clone();
        let key = Folded::new(name);

        let channel_name = match self.channels.get(&key) {
            Some(channel) => {
                if !channel.is_member(id) {
                    return Err(ErrorReply::NotOnChannel(channel.name.clone()));
                }
                if channel.is_member(target_id) {
                    return Err(ErrorReply::UserOnChannel(target_nick, channel.name.clone()));
                }
                if channel.modes.invite_only && !channel.is_op(id) {
                    return Err(ErrorReply::ChanOpPrivsNeeded(channel.name.clone()));
                }
                channel.name.clone()
            }
            // RFC 1459 lets anyone invite anyone to a channel that does not
            // exist yet. Nothing is recorded; it is only a suggestion.
            None => name.clone(),
        };
        if let Some(channel) = self.channels.get_mut(&key) {
            channel.invites.insert(target_id);
        }

        self.send(id, self.reply(id, RPL_INVITING).param(&target_nick).param(&channel_name));
        let invitation = Message::new("INVITE")
            .with_prefix(self.client(id).mask())
            .param(&target_nick)
            .param(&channel_name);
        self.send(target_id, invitation);
        if let Some(away) = away {
            self.send(id, self.reply(id, RPL_AWAY).param(&target_nick).trailing(away));
        }
        Ok(())
    }

    /// `KICK <channel> <user>{,<user>} [<comment>]`
    pub(super) fn kick(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let name = &msg.params[0];
        let key = Folded::new(name);
        let channel =
            self.channels.get(&key).ok_or_else(|| ErrorReply::NoSuchChannel(name.clone()))?;
        if !channel.is_member(id) {
            return Err(ErrorReply::NotOnChannel(channel.name.clone()));
        }
        let client = self.client(id);
        let (mask, nick) = (client.mask(), client.nick().to_owned());
        let reason = msg.arg(2).filter(|r| !r.is_empty()).unwrap_or(&nick);
        let reason = clip(reason, KICKLEN).to_owned();

        for target in msg.params[1].split(',').filter(|t| !t.is_empty()) {
            // Look again each time round: a kicker may kick themselves out.
            let Some(channel) = self.channels.get(&key) else {
                break;
            };
            if !channel.is_op(id) {
                self.send_error(id, ErrorReply::ChanOpPrivsNeeded(channel.name.clone()));
                break;
            }
            let Some(victim) = self.find_nick(target) else {
                self.send_error(id, ErrorReply::NoSuchNick(target.to_owned()));
                continue;
            };
            if !channel.is_member(victim.id) {
                let error =
                    ErrorReply::UserNotInChannel(victim.nick().to_owned(), channel.name.clone());
                self.send_error(id, error);
                continue;
            }
            let victim_id = victim.id;
            let kick = Message::new("KICK")
                .with_prefix(&mask)
                .param(&channel.name)
                .param(victim.nick())
                .trailing(&reason);
            self.send_channel(channel, &Line::from(kick.to_line()), None);
            self.remove_member(&key, victim_id);
        }
        Ok(())
    }
}
