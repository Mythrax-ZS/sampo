//! Channels: who is in them, what they are about, and who may speak.

use std::collections::{BTreeMap, BTreeSet};

use super::client::ClientId;
use crate::proto::casemap;

/// Every channel mode there is, as `RPL_MYINFO` lists them.
pub const ALL_MODES: &str = "biklmnopstv";
/// The channel modes that take a parameter, for the same reply.
pub const PARAM_MODES: &str = "bklov";

/// A channel, from the moment its first member joins until its last leaves.
pub struct Channel {
    /// The name as its creator spelled it.
    pub name: String,
    /// When it was created, in seconds since the epoch.
    pub created: i64,
    pub topic: Option<Topic>,
    pub members: BTreeMap<ClientId, Member>,
    pub modes: ChannelModes,
    pub bans: Vec<Ban>,
    /// Who has been invited, and not yet come in.
    pub invites: BTreeSet<ClientId>,
}

/// A channel's topic, and who set it when.
pub struct Topic {
    pub text: String,
    pub set_by: String,
    pub set_at: i64,
}

/// A ban mask, and who set it when.
pub struct Ban {
    pub mask: String,
    pub set_by: String,
    pub set_at: i64,
}

/// One member's standing in a channel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    /// `+o`: may change modes, topics and memberships.
    pub op: bool,
    /// `+v`: may speak when the channel is moderated.
    pub voice: bool,
}

impl Member {
    /// `@` for an operator, `+` for a voice, as NAMES and WHO show them.
    pub fn prefix(self) -> &'static str {
        if self.op {
            "@"
        } else if self.voice {
            "+"
        } else {
            ""
        }
    }
}

/// A channel's modes, apart from the per-member ones and the ban list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ChannelModes {
    /// `+i`: only the invited may join.
    pub invite_only: bool,
    /// `+m`: only operators and voices may speak.
    pub moderated: bool,
    /// `+n`: no messages from outside the channel.
    pub no_external: bool,
    /// `+p`: private.
    pub private: bool,
    /// `+s`: secret.
    pub secret: bool,
    /// `+t`: only operators may set the topic.
    pub topic_lock: bool,
    /// `+k`: the key needed to join.
    pub key: Option<String>,
    /// `+l`: the most members allowed.
    pub limit: Option<usize>,
}

impl ChannelModes {
    /// The on-off mode a letter stands for, if it stands for one.
    pub fn flag_mut(&mut self, mode: char) -> Option<&mut bool> {
        Some(match mode {
            'i' => &mut self.invite_only,
            'm' => &mut self.moderated,
            'n' => &mut self.no_external,
            'p' => &mut self.private,
            's' => &mut self.secret,
            't' => &mut self.topic_lock,
            _ => return None,
        })
    }

    /// The modes as `RPL_CHANNELMODEIS` gives them: `+ntkl`, then the key
    /// and limit, which only members are shown.
    pub fn describe(&self, reveal: bool) -> (String, Vec<String>) {
        let mut flags = String::from("+");
        let switches = [
            (self.invite_only, 'i'),
            (self.moderated, 'm'),
            (self.no_external, 'n'),
            (self.private, 'p'),
            (self.secret, 's'),
            (self.topic_lock, 't'),
        ];
        flags.extend(switches.iter().filter(|(on, _)| *on).map(|&(_, c)| c));
        let mut args = Vec::new();
        if let Some(key) = &self.key {
            flags.push('k');
            if reveal {
                args.push(key.clone());
            }
        }
        if let Some(limit) = self.limit {
            flags.push('l');
            if reveal {
                args.push(limit.to_string());
            }
        }
        (flags, args)
    }
}

impl Channel {
    /// A new channel, `+nt` as most servers have made them since the 1990s.
    pub fn new(name: &str, now: i64) -> Channel {
        Channel {
            name: name.to_owned(),
            created: now,
            topic: None,
            members: BTreeMap::new(),
            modes: ChannelModes { no_external: true, topic_lock: true, ..ChannelModes::default() },
            bans: Vec::new(),
            invites: BTreeSet::new(),
        }
    }

    pub fn is_member(&self, id: ClientId) -> bool {
        self.members.contains_key(&id)
    }

    pub fn is_op(&self, id: ClientId) -> bool {
        self.members.get(&id).is_some_and(|m| m.op)
    }

    /// Whether the channel is kept from outsiders. Here `+p` and `+s` are
    /// treated alike: such a channel is invisible to anyone not in it.
    pub fn is_hidden(&self) -> bool {
        self.modes.secret || self.modes.private
    }

    /// Whether a `nick!user@host` mask matches one of the bans.
    pub fn is_banned(&self, mask: &str) -> bool {
        self.bans.iter().any(|ban| casemap::wildcard(&ban.mask, mask))
    }

    /// Whether a client may send to the channel. Operators and voices
    /// always may; for anyone else, bans, `+m` and `+n` have their say.
    pub fn can_speak(&self, id: ClientId, mask: &str) -> bool {
        match self.members.get(&id) {
            Some(member) if member.op || member.voice => true,
            Some(_) => !self.modes.moderated && !self.is_banned(mask),
            None => !self.modes.no_external && !self.modes.moderated && !self.is_banned(mask),
        }
    }

    /// The channel's kind as `RPL_NAMREPLY` marks it: `@` secret, `*`
    /// private, `=` public.
    pub fn kind(&self) -> &'static str {
        if self.modes.secret {
            "@"
        } else if self.modes.private {
            "*"
        } else {
            "="
        }
    }
}
