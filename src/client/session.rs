//! What the client knows of the server: who it is there, which channels
//! it is in, and who else is in them.

use std::collections::BTreeMap;

use crate::proto::{Folded, casemap};

/// The client's side of a connection.
#[derive(Debug)]
pub struct Session {
    /// Our nickname: as the server last confirmed it, or as we asked.
    pub nick: String,
    pub registered: bool,
    /// The server's name, from its welcome.
    pub server: Option<String>,
    /// Our user modes, as letters.
    pub modes: String,
    pub away: bool,
    /// Our `nick!user@host` as the server sees it, once we know.
    pub mask: Option<String>,
    pub channels: BTreeMap<Folded, Channel>,
    /// Channels and private conversations, in the order they were opened:
    /// what Ctrl-X goes round.
    pub targets: Vec<String>,
    /// Where typed text goes.
    pub current: Option<String>,
    /// Targets with something new in them that we have not looked at.
    pub activity: Vec<String>,
    /// Whoever last sent us a private message, for `/msg ,`.
    pub last_from: Option<String>,
    /// Whoever we last sent one to, for `/msg .`.
    pub last_to: Option<String>,
    /// Channels to join once the server welcomes us.
    pub rejoin: Vec<String>,
    /// How many other nicknames we have tried while registering.
    pub nick_tries: u32,
    pub support: Support,
}

/// A channel we are in.
#[derive(Debug, Clone)]
pub struct Channel {
    pub name: String,
    pub topic: Option<String>,
    /// The channel's on-off modes, as letters.
    pub modes: String,
    pub members: BTreeMap<Folded, Member>,
}

/// Someone in a channel.
#[derive(Debug, Clone)]
pub struct Member {
    pub nick: String,
    /// The status modes they hold there, such as `o` and `v`.
    pub modes: String,
}

/// What the server says it supports in `RPL_ISUPPORT`; RFC 1459 until it
/// says otherwise.
#[derive(Debug, Clone)]
pub struct Support {
    pub chantypes: String,
    /// The status modes, most powerful first, and the signs that mark them.
    pub prefix: Vec<(char, char)>,
    /// Channel modes by kind: lists, those that always take a parameter,
    /// those that take one only when set, and plain switches.
    pub chanmodes: [String; 4],
}

impl Default for Support {
    fn default() -> Self {
        Support {
            chantypes: String::from("#&"),
            prefix: vec![('o', '@'), ('v', '+')],
            chanmodes: [
                String::from("b"),
                String::from("k"),
                String::from("l"),
                String::from("imnpst"),
            ],
        }
    }
}

impl Support {
    /// Takes in a batch of `RPL_ISUPPORT` tokens.
    pub fn update(&mut self, tokens: &[String]) {
        for token in tokens {
            let (name, value) = token.split_once('=').unwrap_or((token, ""));
            match name {
                "CHANTYPES" if !value.is_empty() => self.chantypes = value.to_owned(),
                "PREFIX" => {
                    let Some((modes, signs)) =
                        value.strip_prefix('(').and_then(|v| v.split_once(')'))
                    else {
                        continue;
                    };
                    self.prefix = modes.chars().zip(signs.chars()).collect();
                }
                "CHANMODES" => {
                    let mut kinds = value.split(',').map(String::from);
                    self.chanmodes = std::array::from_fn(|_| kinds.next().unwrap_or_default());
                }
                _ => {}
            }
        }
    }
}

impl Session {
    /// A session not yet connected, that will join `channels` once it is.
    pub fn new(nick: &str, channels: &[String]) -> Session {
        Session {
            nick: nick.to_owned(),
            registered: false,
            server: None,
            modes: String::new(),
            away: false,
            mask: None,
            channels: BTreeMap::new(),
            targets: Vec::new(),
            current: None,
            activity: Vec::new(),
            last_from: None,
            last_to: None,
            rejoin: channels.to_vec(),
            nick_tries: 0,
            support: Support::default(),
        }
    }

    /// Forgets the connection, remembering which channels to rejoin.
    pub fn disconnected(&mut self) {
        let channels = self.channels.values().map(|c| c.name.clone());
        let mut rejoin: Vec<String> = channels.collect();
        rejoin.retain(|name| !self.rejoin.iter().any(|r| casemap::equal(r, name)));
        self.rejoin.append(&mut rejoin);
        self.targets.retain(|t| !self.support.chantypes.contains(first(t)));
        self.channels.clear();
        self.current = self.current.take().filter(|t| self.targets.contains(t));
        self.activity.clear();
        self.registered = false;
        self.server = None;
        self.modes.clear();
        self.away = false;
        self.mask = None;
        self.nick_tries = 0;
    }

    /// Whether a target is a channel rather than a nickname.
    pub fn is_channel(&self, target: &str) -> bool {
        self.support.chantypes.contains(first(target))
    }

    /// Whether `target` is where typed text goes.
    pub fn is_current(&self, target: &str) -> bool {
        self.current.as_deref().is_some_and(|c| casemap::equal(c, target))
    }

    /// The current target, if it is a channel.
    pub fn current_channel(&self) -> Option<&str> {
        self.current.as_deref().filter(|c| self.is_channel(c))
    }

    pub fn channel(&self, name: &str) -> Option<&Channel> {
        self.channels.get(&Folded::new(name))
    }

    /// We joined a channel: it becomes the current one.
    pub fn joined(&mut self, name: &str) {
        let channel = Channel {
            name: name.to_owned(),
            topic: None,
            modes: String::new(),
            members: BTreeMap::new(),
        };
        self.channels.insert(Folded::new(name), channel);
        self.add_target(name);
        self.switch_to(name);
    }

    /// We left a channel, or were made to.
    pub fn parted(&mut self, name: &str) {
        self.channels.remove(&Folded::new(name));
        self.close(name);
    }

    /// Opens a private conversation, making it current if `focus`.
    pub fn open_query(&mut self, nick: &str, focus: bool) {
        self.add_target(nick);
        if focus {
            self.switch_to(nick);
        }
    }

    fn add_target(&mut self, target: &str) {
        if !self.targets.iter().any(|t| casemap::equal(t, target)) {
            self.targets.push(target.to_owned());
        }
    }

    /// Closes a channel or conversation, moving on to the next if it was
    /// current.
    pub fn close(&mut self, target: &str) {
        let Some(index) = self.targets.iter().position(|t| casemap::equal(t, target)) else {
            return;
        };
        self.targets.remove(index);
        self.activity.retain(|t| !casemap::equal(t, target));
        if self.is_current(target) {
            match self.targets.get(index).or(self.targets.last()).cloned() {
                Some(next) => self.switch_to(&next),
                None => self.current = None,
            }
        }
    }

    /// Makes `target` current, and its news seen.
    pub fn switch_to(&mut self, target: &str) {
        self.activity.retain(|t| !casemap::equal(t, target));
        self.current = Some(target.to_owned());
    }

    /// Moves `step` places round the targets, and returns the one arrived at.
    pub fn cycle(&mut self, step: isize) -> Option<String> {
        if self.targets.is_empty() {
            return None;
        }
        let len = self.targets.len() as isize;
        let here = self
            .current
            .as_deref()
            .and_then(|c| self.targets.iter().position(|t| casemap::equal(t, c)))
            .map_or(-1, |i| i as isize);
        let next = self.targets[(here + step).rem_euclid(len) as usize].clone();
        self.switch_to(&next);
        Some(next)
    }

    /// Notes news in a target we are not looking at.
    pub fn mark_activity(&mut self, target: &str) {
        if !self.is_current(target) && !self.activity.iter().any(|t| casemap::equal(t, target)) {
            self.activity.push(target.to_owned());
        }
    }

    pub fn set_topic(&mut self, channel: &str, topic: &str) {
        if let Some(channel) = self.channels.get_mut(&Folded::new(channel)) {
            channel.topic = (!topic.is_empty()).then(|| topic.to_owned());
        }
    }

    /// Someone joined a channel we are in.
    pub fn add_member(&mut self, channel: &str, nick: &str, modes: &str) {
        if let Some(channel) = self.channels.get_mut(&Folded::new(channel)) {
            let member = Member { nick: nick.to_owned(), modes: modes.to_owned() };
            channel.members.insert(Folded::new(nick), member);
        }
    }

    /// Someone left a channel we are in.
    pub fn remove_member(&mut self, channel: &str, nick: &str) {
        if let Some(channel) = self.channels.get_mut(&Folded::new(channel)) {
            channel.members.remove(&Folded::new(nick));
        }
    }

    /// Takes in a line of `RPL_NAMREPLY`: names, each with its status signs.
    pub fn add_names(&mut self, channel: &str, names: &str) {
        for name in names.split(' ').filter(|n| !n.is_empty()) {
            let nick = name.trim_start_matches(|c| self.prefix_mode(c).is_some());
            let modes: String = name[..name.len() - nick.len()]
                .chars()
                .filter_map(|c| self.prefix_mode(c))
                .collect();
            self.add_member(channel, nick, &modes);
        }
    }

    /// The status mode a sign such as `@` stands for.
    fn prefix_mode(&self, sign: char) -> Option<char> {
        self.support.prefix.iter().find(|&&(_, s)| s == sign).map(|&(mode, _)| mode)
    }

    /// The sign for the most powerful status a member holds, if any.
    pub fn prefix(&self, member: &Member) -> Option<char> {
        self.support
            .prefix
            .iter()
            .find(|(mode, _)| member.modes.contains(*mode))
            .map(|&(_, sign)| sign)
    }

    /// Our own sign in a channel, such as `@`.
    pub fn my_prefix(&self, channel: &str) -> Option<char> {
        let member = self.channel(channel)?.members.get(&Folded::new(&self.nick))?;
        self.prefix(member)
    }

    /// Someone quit. Returns whether they were anywhere we would notice.
    pub fn quit(&mut self, nick: &str) -> bool {
        let key = Folded::new(nick);
        let mut seen = false;
        for channel in self.channels.values_mut() {
            seen |= channel.members.remove(&key).is_some();
        }
        seen || self.targets.iter().any(|t| casemap::equal(t, nick))
    }

    /// Someone changed their nickname. Returns whether we would notice.
    pub fn rename(&mut self, old: &str, new: &str) -> bool {
        let (old_key, new_key) = (Folded::new(old), Folded::new(new));
        let mut seen = false;
        for channel in self.channels.values_mut() {
            if let Some(mut member) = channel.members.remove(&old_key) {
                member.nick = new.to_owned();
                channel.members.insert(new_key.clone(), member);
                seen = true;
            }
        }
        let renamed = |name: &mut String| {
            if casemap::equal(name, old) {
                *name = new.to_owned();
                true
            } else {
                false
            }
        };
        for target in &mut self.targets {
            seen |= renamed(target);
        }
        self.activity.iter_mut().for_each(|t| {
            renamed(t);
        });
        for name in
            [&mut self.current, &mut self.last_from, &mut self.last_to].into_iter().flatten()
        {
            renamed(name);
        }
        seen
    }

    /// Applies a channel's MODE change to what we know of it.
    pub fn apply_channel_modes(&mut self, channel: &str, modes: &str, args: &[String]) {
        let Some(channel) = self.channels.get_mut(&Folded::new(channel)) else {
            return;
        };
        let support = &self.support;
        let mut args = args.iter();
        let mut adding = true;
        for mode in modes.chars() {
            if mode == '+' || mode == '-' {
                adding = mode == '+';
            } else if support.prefix.iter().any(|&(m, _)| m == mode) {
                let Some(nick) = args.next() else {
                    continue;
                };
                if let Some(member) = channel.members.get_mut(&Folded::new(nick)) {
                    set_letter(&mut member.modes, mode, adding);
                }
            } else if support.chanmodes[0].contains(mode) {
                args.next();
            } else if support.chanmodes[1].contains(mode) {
                args.next();
                set_letter(&mut channel.modes, mode, adding);
            } else if support.chanmodes[2].contains(mode) {
                if adding {
                    args.next();
                }
                set_letter(&mut channel.modes, mode, adding);
            } else {
                set_letter(&mut channel.modes, mode, adding);
            }
        }
    }

    /// Sets a channel's modes from `RPL_CHANNELMODEIS`.
    pub fn set_channel_modes(&mut self, channel: &str, modes: &str) {
        if let Some(channel) = self.channels.get_mut(&Folded::new(channel)) {
            channel.modes.clear();
            for mode in modes.chars().filter(|&m| m != '+') {
                set_letter(&mut channel.modes, mode, true);
            }
        }
    }

    /// Applies a change to our own user modes.
    pub fn apply_user_modes(&mut self, modes: &str) {
        let mut adding = true;
        for mode in modes.chars() {
            match mode {
                '+' | '-' => adding = mode == '+',
                _ => set_letter(&mut self.modes, mode, adding),
            }
        }
    }

    /// The nicks in the current channel, for Tab.
    pub fn current_members(&self) -> Vec<String> {
        self.current_channel()
            .and_then(|name| self.channel(name))
            .map(|channel| channel.members.values().map(|m| m.nick.clone()).collect())
            .unwrap_or_default()
    }
}

/// The first character of a name, or a space for an empty one.
fn first(name: &str) -> char {
    name.chars().next().unwrap_or(' ')
}

/// Puts a mode letter into a set of them, or takes it out, keeping the
/// set in order.
fn set_letter(modes: &mut String, mode: char, on: bool) {
    let mut letters: Vec<char> = modes.chars().filter(|&m| m != mode).collect();
    if on {
        letters.push(mode);
        letters.sort_unstable();
    }
    *modes = letters.into_iter().collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_channel() -> Session {
        let mut session = Session::new("alice", &[]);
        session.joined("#sampo");
        session.add_names("#sampo", "@alice +bob carol");
        session
    }

    #[test]
    fn names_carry_status() {
        let session = in_channel();
        let channel = session.channel("#SAMPO").unwrap();
        assert_eq!(channel.members.len(), 3);
        assert_eq!(session.my_prefix("#sampo"), Some('@'));
        let bob = &channel.members[&Folded::new("bob")];
        assert_eq!(session.prefix(bob), Some('+'));
    }

    #[test]
    fn modes_are_followed() {
        let mut session = in_channel();
        let args =
            ["carol".to_string(), "secret".to_string(), "10".to_string(), "alice".to_string()];
        session.apply_channel_modes("#sampo", "+okl-o+m", &args);
        let channel = session.channel("#sampo").unwrap();
        assert_eq!(channel.modes, "klm");
        assert_eq!(session.my_prefix("#sampo"), None);
        assert_eq!(channel.members[&Folded::new("carol")].modes, "o");
    }

    #[test]
    fn renames_reach_everywhere() {
        let mut session = in_channel();
        session.open_query("bob", true);
        session.last_from = Some("Bob".into());
        assert!(session.rename("bob", "robert"));
        assert!(session.is_current("robert"));
        assert_eq!(session.last_from.as_deref(), Some("robert"));
        assert!(session.channel("#sampo").unwrap().members.contains_key(&Folded::new("robert")));
    }

    #[test]
    fn cycling_goes_round() {
        let mut session = in_channel();
        session.joined("#rust");
        session.open_query("bob", false);
        assert_eq!(session.cycle(1).as_deref(), Some("bob"));
        assert_eq!(session.cycle(1).as_deref(), Some("#sampo"));
        assert_eq!(session.cycle(-1).as_deref(), Some("bob"));
    }

    #[test]
    fn leaving_moves_on() {
        let mut session = in_channel();
        session.joined("#rust");
        session.mark_activity("#sampo");
        session.parted("#rust");
        assert!(session.is_current("#sampo"));
        session.switch_to("#sampo");
        assert!(session.activity.is_empty());
    }

    #[test]
    fn disconnecting_remembers_channels() {
        let mut session = Session::new("alice", &["#sampo".to_string()]);
        session.rejoin.clear();
        session.joined("#sampo");
        session.open_query("bob", false);
        session.disconnected();
        assert_eq!(session.rejoin, ["#sampo"]);
        assert_eq!(session.targets, ["bob"]);
        assert!(session.channels.is_empty());
    }
}
