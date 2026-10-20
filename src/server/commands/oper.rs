//! What IRC operators may do: become one, remove a user, address everyone
//! who listens, and bring the server down.

use super::{Outcome, clip};
use crate::proto::Message;
use crate::proto::numeric::RPL_YOUREOPER;
use crate::server::client::ClientId;
use crate::server::{ErrorReply, KICKLEN, Line, Server};

impl Server {
    /// `OPER <name> <password>`
    pub(super) fn oper(&mut self, id: ClientId, msg: &Message) -> Outcome {
        if self.config.opers.is_empty() {
            return Err(ErrorReply::NoOperHost);
        }
        let (name, password) = (&msg.params[0], &msg.params[1]);
        let known =
            self.config.opers.iter().any(|oper| &oper.name == name && &oper.password == password);
        if !known {
            log!("#{id} {} failed OPER as {name}", self.client(id).mask());
            return Err(ErrorReply::PasswdMismatch);
        }
        let client = self.client_mut(id);
        if client.modes.operator {
            return Ok(());
        }
        client.modes.operator = true;
        let (nick, mask) = (client.nick().to_owned(), client.mask());
        self.send(id, self.reply(id, RPL_YOUREOPER).trailing("You are now an IRC operator"));
        self.send(id, Message::new("MODE").with_prefix(&nick).param(&nick).trailing("+o"));
        log!("#{id} {mask} is now an operator, as {name}");
        Ok(())
    }

    /// `KILL <nick> [<reason>]`: disconnects a user.
    pub(super) fn kill(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let killer = self.client(id);
        if !killer.modes.operator {
            return Err(ErrorReply::NoPrivileges);
        }
        let (killer_nick, killer_mask) = (killer.nick().to_owned(), killer.mask());
        let target = &msg.params[0];
        let victim =
            self.find_nick(target).ok_or_else(|| ErrorReply::NoSuchNick(target.clone()))?;
        let victim_id = victim.id;
        let reason = msg.arg(1).filter(|r| !r.is_empty()).unwrap_or("No reason");
        let reason = format!("{killer_nick} ({})", clip(reason, KICKLEN));
        let kill =
            Message::new("KILL").with_prefix(&killer_mask).param(victim.nick()).trailing(&reason);
        self.send(victim_id, kill);
        log!("#{id} {killer_mask} killed {} ({reason})", victim.mask());
        self.exit_client(victim_id, &format!("Killed ({reason})"));
        Ok(())
    }

    /// `WALLOPS <text>`: a message to every user who is `+w`.
    pub(super) fn wallops(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let sender = self.client(id);
        if !sender.modes.operator {
            return Err(ErrorReply::NoPrivileges);
        }
        let wallops = Message::new("WALLOPS").with_prefix(sender.mask()).trailing(&msg.params[0]);
        let wallops = Line::from(wallops.to_line());
        for client in self.clients.values() {
            if client.registered && client.modes.wallops {
                self.send_line(client.id, &wallops);
            }
        }
        Ok(())
    }

    /// `DIE`: shuts the server down.
    pub(super) fn die(&mut self, id: ClientId, _: &Message) -> Outcome {
        let client = self.client(id);
        if !client.modes.operator {
            return Err(ErrorReply::NoPrivileges);
        }
        self.dying = Some(format!("Received DIE from {}", client.mask()));
        Ok(())
    }
}
