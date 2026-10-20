//! PRIVMSG and NOTICE: what IRC is for.

use std::collections::BTreeSet;

use tokio::time::Instant;

use super::Outcome;
use crate::proto::numeric::RPL_AWAY;
use crate::proto::{Folded, Message};
use crate::server::client::ClientId;
use crate::server::{ErrorReply, Line, MAX_TARGETS, Server};

impl Server {
    /// `PRIVMSG <target>{,<target>} <text>`
    pub(super) fn privmsg(&mut self, id: ClientId, msg: &Message) -> Outcome {
        self.client_mut(id).last_spoke = Instant::now();
        self.deliver(id, msg, "PRIVMSG")
    }

    /// `NOTICE <target>{,<target>} <text>`: like PRIVMSG, except that no
    /// automatic reply may ever be sent to one, errors included (RFC 1459
    /// §4.4.2). That is what keeps two robots from talking forever.
    pub(super) fn notice(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let _ = self.deliver(id, msg, "NOTICE");
        Ok(())
    }

    fn deliver(&self, id: ClientId, msg: &Message, kind: &'static str) -> Outcome {
        let quiet = kind == "NOTICE";
        let targets = msg.arg(0).filter(|t| !t.is_empty());
        let Some(targets) = targets else {
            return Err(ErrorReply::NoRecipient(kind));
        };
        let Some(text) = msg.arg(1).filter(|t| !t.is_empty()) else {
            return Err(ErrorReply::NoTextToSend);
        };
        let source = self.client(id).mask();
        let mut seen = BTreeSet::new();
        for target in targets.split(',').filter(|t| !t.is_empty()).take(MAX_TARGETS) {
            if !seen.insert(Folded::new(target)) {
                continue;
            }
            let delivered = if target.starts_with('#') {
                self.to_channel(id, &source, target, text, kind)
            } else {
                self.to_user(id, &source, target, text, kind)
            };
            if let Err(error) = delivered
                && !quiet
            {
                self.send_error(id, error);
            }
        }
        Ok(())
    }

    fn to_channel(
        &self,
        id: ClientId,
        source: &str,
        target: &str,
        text: &str,
        kind: &str,
    ) -> Outcome {
        let channel = self
            .channels
            .get(&Folded::new(target))
            .ok_or_else(|| ErrorReply::NoSuchNick(target.to_owned()))?;
        if !channel.can_speak(id, source) {
            return Err(ErrorReply::CannotSendToChan(channel.name.clone()));
        }
        let line = Message::new(kind).with_prefix(source).param(&channel.name).trailing(text);
        self.send_channel(channel, &Line::from(line.to_line()), Some(id));
        Ok(())
    }

    fn to_user(&self, id: ClientId, source: &str, target: &str, text: &str, kind: &str) -> Outcome {
        let recipient =
            self.find_nick(target).ok_or_else(|| ErrorReply::NoSuchNick(target.to_owned()))?;
        let line = Message::new(kind).with_prefix(source).param(recipient.nick()).trailing(text);
        self.send(recipient.id, line);
        if let Some(away) = &recipient.away
            && kind == "PRIVMSG"
        {
            self.send(id, self.reply(id, RPL_AWAY).param(recipient.nick()).trailing(away));
        }
        Ok(())
    }
}
