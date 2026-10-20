//! MODE, for users and for channels.

use super::{Outcome, clip, now};
use crate::proto::numeric::*;
use crate::proto::{Folded, Message, casemap};
use crate::server::channel::{Ban, Channel};
use crate::server::client::ClientId;
use crate::server::{ErrorReply, KEYLEN, Line, MAX_BANS, MAX_MODES, Server};

impl Server {
    /// `MODE <nickname> [<modes>]`, or
    /// `MODE <channel> [<modes> [<parameters>]]`.
    pub(super) fn mode(&mut self, id: ClientId, msg: &Message) -> Outcome {
        if msg.params[0].starts_with('#') {
            self.channel_mode(id, msg)
        } else {
            self.user_mode(id, msg)
        }
    }

    fn user_mode(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let target = &msg.params[0];
        let owner = self.find_nick(target).ok_or_else(|| ErrorReply::NoSuchNick(target.clone()))?;
        if owner.id != id {
            return Err(ErrorReply::UsersDontMatch);
        }
        let Some(requested) = msg.arg(1) else {
            let modes = self.client(id).modes.to_string();
            self.send(id, self.reply(id, RPL_UMODEIS).param(modes));
            return Ok(());
        };

        let modes = &mut self.client_mut(id).modes;
        let mut changes = ModeChanges::default();
        let mut unknown = false;
        let mut adding = true;
        for mode in requested.chars() {
            let flag = match mode {
                '+' | '-' => {
                    adding = mode == '+';
                    continue;
                }
                'i' => &mut modes.invisible,
                'w' => &mut modes.wallops,
                // Operator status is had from OPER alone, but may be given up.
                'o' if !adding => &mut modes.operator,
                'o' => continue,
                _ => {
                    unknown = true;
                    continue;
                }
            };
            if *flag != adding {
                *flag = adding;
                changes.push(adding, mode, None);
            }
        }

        if unknown {
            self.send_error(id, ErrorReply::UmodeUnknownFlag);
        }
        if !changes.is_empty() {
            let nick = self.client(id).nick();
            let (modes, _) = changes.render();
            self.send(id, Message::new("MODE").with_prefix(nick).param(nick).trailing(modes));
        }
        Ok(())
    }

    fn channel_mode(&mut self, id: ClientId, msg: &Message) -> Outcome {
        let name = &msg.params[0];
        let key = Folded::new(name);
        let channel =
            self.channels.get(&key).ok_or_else(|| ErrorReply::NoSuchChannel(name.clone()))?;
        let Some(requested) = msg.arg(1) else {
            let (modes, args) = channel.modes.describe(channel.is_member(id));
            let reply = self.reply(id, RPL_CHANNELMODEIS).param(&channel.name).param(modes);
            self.send(id, reply.params(args));
            let created = channel.created.to_string();
            self.send(id, self.reply(id, RPL_CREATIONTIME).param(&channel.name).param(created));
            return Ok(());
        };

        let is_op = channel.is_op(id);
        let client = self.client(id);
        let (mask, setter) = (client.mask(), client.nick().to_owned());
        let (nicks, clients) = (&self.nicks, &self.clients);
        let resolve = |nick: &str| {
            let id = nicks.get(&Folded::new(nick))?;
            let client = clients.get(id).filter(|c| c.registered)?;
            Some((client.id, client.nick().to_owned()))
        };
        let Some(channel) = self.channels.get_mut(&key) else {
            return Ok(());
        };
        let request = Request {
            is_op,
            modes: requested,
            args: &msg.params[2..],
            setter: &setter,
            now: now(),
        };
        let applied = apply(channel, &request, resolve);

        for error in applied.errors {
            self.send_error(id, error);
        }
        if applied.list_bans {
            self.send_ban_list(id, &key);
        }
        if !applied.changes.is_empty()
            && let Some(channel) = self.channels.get(&key)
        {
            let (modes, args) = applied.changes.render();
            let change = Message::new("MODE")
                .with_prefix(mask)
                .param(&channel.name)
                .param(modes)
                .params(args);
            self.send_channel(channel, &Line::from(change.to_line()), None);
        }
        Ok(())
    }

    fn send_ban_list(&self, id: ClientId, key: &Folded) {
        let Some(channel) = self.channels.get(key) else {
            return;
        };
        for ban in &channel.bans {
            let set_at = ban.set_at.to_string();
            let entry =
                [channel.name.as_str(), ban.mask.as_str(), ban.set_by.as_str(), set_at.as_str()];
            self.send(id, self.reply(id, RPL_BANLIST).params(entry));
        }
        let end = self.reply(id, RPL_ENDOFBANLIST).param(&channel.name);
        self.send(id, end.trailing("End of channel ban list"));
    }
}

/// A MODE command on a channel, as far as the channel needs to know.
struct Request<'a> {
    /// Whether the sender is a channel operator.
    is_op: bool,
    modes: &'a str,
    args: &'a [String],
    /// Who to record as the setter of any ban.
    setter: &'a str,
    now: i64,
}

/// What came of a MODE command on a channel.
#[derive(Debug, Default)]
struct Applied {
    /// What actually changed, to be told to the channel.
    changes: ModeChanges,
    /// What to tell the sender went wrong.
    errors: Vec<ErrorReply>,
    /// Whether the sender asked to see the ban list.
    list_bans: bool,
}

/// Hands out a MODE command's parameters to the modes that need them, and
/// keeps note of whether the sender overstepped.
struct Params<'a> {
    rest: std::slice::Iter<'a, String>,
    is_op: bool,
    taken: usize,
    /// Whether someone other than an operator tried to change something.
    denied: bool,
}

impl Params<'_> {
    /// The parameter for the next mode: consumed whatever happens, but
    /// only handed over if the sender is an operator and the command still
    /// has room for another change with a parameter.
    fn take(&mut self) -> Option<String> {
        let arg = self.rest.next()?;
        if !self.allowed() || self.taken == MAX_MODES {
            return None;
        }
        self.taken += 1;
        Some(arg.clone())
    }

    /// Whether the sender may change modes; noting it if not.
    fn allowed(&mut self) -> bool {
        self.denied |= !self.is_op;
        self.is_op
    }

    fn is_empty(&self) -> bool {
        self.rest.len() == 0
    }
}

/// Applies a mode string to a channel, ircd fashion: anything malformed is
/// skipped quietly, every refusal is reported, and only real changes are
/// counted. `resolve` looks up a nickname for `+o` and `+v`.
fn apply(
    channel: &mut Channel,
    request: &Request,
    resolve: impl Fn(&str) -> Option<(ClientId, String)>,
) -> Applied {
    let mut out = Applied::default();
    let mut params =
        Params { rest: request.args.iter(), is_op: request.is_op, taken: 0, denied: false };
    let mut adding = true;

    for mode in request.modes.chars() {
        match mode {
            '+' | '-' => adding = mode == '+',
            'b' => {
                // A bare `b` asks to see the list, which anyone may.
                if params.is_empty() {
                    out.list_bans = true;
                    continue;
                }
                let Some(mask) = params.take() else {
                    continue;
                };
                let mask = normalize_mask(&mask);
                let existing = channel.bans.iter().position(|b| casemap::equal(&b.mask, &mask));
                match (adding, existing) {
                    (true, None) if channel.bans.len() >= MAX_BANS => {
                        out.errors.push(ErrorReply::BanListFull(channel.name.clone()));
                    }
                    (true, None) => {
                        channel.bans.push(Ban {
                            mask: mask.clone(),
                            set_by: request.setter.to_owned(),
                            set_at: request.now,
                        });
                        out.changes.push(true, 'b', Some(mask));
                    }
                    (false, Some(index)) => {
                        let ban = channel.bans.remove(index);
                        out.changes.push(false, 'b', Some(ban.mask));
                    }
                    _ => {}
                }
            }
            'o' | 'v' => {
                let Some(nick) = params.take() else {
                    continue;
                };
                let Some((target, nick)) = resolve(&nick) else {
                    out.errors.push(ErrorReply::NoSuchNick(nick));
                    continue;
                };
                let Some(member) = channel.members.get_mut(&target) else {
                    out.errors.push(ErrorReply::UserNotInChannel(nick, channel.name.clone()));
                    continue;
                };
                let flag = if mode == 'o' { &mut member.op } else { &mut member.voice };
                if *flag != adding {
                    *flag = adding;
                    out.changes.push(adding, mode, Some(nick));
                }
            }
            // `+k` and `-k` both take a parameter; `-k` ignores its value.
            'k' => {
                let Some(key) = params.take() else {
                    continue;
                };
                if !adding {
                    if channel.modes.key.take().is_some() {
                        out.changes.push(false, 'k', Some("*".into()));
                    }
                    continue;
                }
                let key = clip(&key, KEYLEN).to_owned();
                if channel.modes.key.is_some() {
                    out.errors.push(ErrorReply::KeySet(channel.name.clone()));
                } else {
                    channel.modes.key = Some(key.clone());
                    out.changes.push(true, 'k', Some(key));
                }
            }
            // `+l` takes a parameter; `-l` does not.
            'l' if adding => {
                let Some(limit) = params.take() else {
                    continue;
                };
                let Ok(limit) = limit.parse::<usize>() else {
                    continue;
                };
                if limit > 0 && channel.modes.limit != Some(limit) {
                    channel.modes.limit = Some(limit);
                    out.changes.push(true, 'l', Some(limit.to_string()));
                }
            }
            'l' => {
                if params.allowed() && channel.modes.limit.take().is_some() {
                    out.changes.push(false, 'l', None);
                }
            }
            _ => match channel.modes.flag_mut(mode) {
                Some(flag) => {
                    if params.allowed() && *flag != adding {
                        *flag = adding;
                        out.changes.push(adding, mode, None);
                    }
                }
                None => {
                    let error = ErrorReply::UnknownMode(mode);
                    if !out.errors.contains(&error) {
                        out.errors.push(error);
                    }
                }
            },
        }
    }
    if params.denied {
        out.errors.push(ErrorReply::ChanOpPrivsNeeded(channel.name.clone()));
    }
    out
}

/// Mode changes in the order they were made, to be echoed as `+o-v a b`.
#[derive(Debug, Default)]
struct ModeChanges(Vec<(bool, char, Option<String>)>);

impl ModeChanges {
    fn push(&mut self, adding: bool, mode: char, arg: Option<String>) {
        self.0.push((adding, mode, arg));
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The changes as a mode string and its parameters.
    fn render(&self) -> (String, Vec<String>) {
        let mut modes = String::new();
        let mut args = Vec::new();
        let mut sign = None;
        for (adding, mode, arg) in &self.0 {
            if sign != Some(*adding) {
                modes.push(if *adding { '+' } else { '-' });
                sign = Some(*adding);
            }
            modes.push(*mode);
            args.extend(arg.iter().cloned());
        }
        (modes, args)
    }
}

/// Fills a ban mask out to the full `nick!user@host`, as ircd's
/// `pretty_mask()` did: `alice` bans `alice!*@*`, and `*@10.0.0.7` bans
/// `*!*@10.0.0.7`.
fn normalize_mask(mask: &str) -> String {
    let (nick, userhost) = match mask.split_once('!') {
        Some(parts) => parts,
        None if mask.contains('@') => ("*", mask),
        None => (mask, "*"),
    };
    let (user, host) = userhost.split_once('@').unwrap_or((userhost, "*"));
    let part = |text: &str| match clip(text, 64) {
        "" => "*".to_owned(),
        text => text.to_owned(),
    };
    format!("{}!{}@{}", part(nick), part(user), part(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(is_op: bool, modes: &'a str, args: &'a [String]) -> Request<'a> {
        Request { is_op, modes, args, setter: "alice", now: 0 }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    /// A channel with alice (1, an operator) and bob (2) in it.
    fn channel() -> Channel {
        let mut channel = Channel::new("#sampo", 0);
        channel.members.insert(1, crate::server::channel::Member { op: true, voice: false });
        channel.members.insert(2, Default::default());
        channel
    }

    fn resolve(nick: &str) -> Option<(ClientId, String)> {
        match nick {
            "alice" => Some((1, "alice".into())),
            "bob" => Some((2, "bob".into())),
            "carol" => Some((3, "carol".into())),
            _ => None,
        }
    }

    #[test]
    fn masks_are_filled_out() {
        assert_eq!(normalize_mask("alice"), "alice!*@*");
        assert_eq!(normalize_mask("*@10.0.0.7"), "*!*@10.0.0.7");
        assert_eq!(normalize_mask("alice!~a"), "alice!~a@*");
        assert_eq!(normalize_mask("a!b@c"), "a!b@c");
        assert_eq!(normalize_mask("!@"), "*!*@*");
    }

    #[test]
    fn changes_render_compactly() {
        let mut changes = ModeChanges::default();
        changes.push(true, 'o', Some("bob".into()));
        changes.push(true, 't', None);
        changes.push(false, 'v', Some("carol".into()));
        changes.push(false, 'k', Some("*".into()));
        assert_eq!(changes.render(), ("+ot-vk".into(), args(&["bob", "carol", "*"])));
    }

    #[test]
    fn operators_change_modes() {
        let mut chan = channel();
        let list = args(&["bob", "bob", "secret", "10"]);
        let applied = apply(&mut chan, &request(true, "+ovkl-n", &list), resolve);
        assert!(applied.errors.is_empty(), "{:?}", applied.errors);
        assert_eq!(
            applied.changes.render(),
            ("+ovkl-n".into(), args(&["bob", "bob", "secret", "10"]))
        );
        assert!(chan.members[&2].op && chan.members[&2].voice);
        assert_eq!(chan.modes.key.as_deref(), Some("secret"));
        assert_eq!(chan.modes.limit, Some(10));
        assert!(!chan.modes.no_external);
    }

    #[test]
    fn others_are_refused_once() {
        let mut chan = channel();
        let list = args(&["alice"]);
        let applied = apply(&mut chan, &request(false, "+im-o", &list), resolve);
        assert!(applied.changes.is_empty());
        assert_eq!(applied.errors, [ErrorReply::ChanOpPrivsNeeded("#sampo".into())]);
        assert!(chan.members[&1].op);
    }

    #[test]
    fn anyone_may_see_the_ban_list() {
        let mut chan = channel();
        let applied = apply(&mut chan, &request(false, "b", &[]), resolve);
        assert!(applied.list_bans && applied.errors.is_empty());
    }

    #[test]
    fn bans_come_and_go() {
        let mut chan = channel();
        apply(&mut chan, &request(true, "+b", &args(&["*@10.0.0.7"])), resolve);
        assert!(chan.is_banned("bob!~bob@10.0.0.7"));
        let applied = apply(&mut chan, &request(true, "-b", &args(&["*!*@10.0.0.7"])), resolve);
        assert_eq!(applied.changes.render(), ("-b".into(), args(&["*!*@10.0.0.7"])));
        assert!(!chan.is_banned("bob!~bob@10.0.0.7"));
    }

    #[test]
    fn a_key_must_be_removed_before_another_is_set() {
        let mut chan = channel();
        apply(&mut chan, &request(true, "+k", &args(&["one"])), resolve);
        let applied = apply(&mut chan, &request(true, "+k", &args(&["two"])), resolve);
        assert_eq!(applied.errors, [ErrorReply::KeySet("#sampo".into())]);
        assert_eq!(chan.modes.key.as_deref(), Some("one"));
    }

    #[test]
    fn strangers_and_unknowns_are_reported() {
        let mut chan = channel();
        let list = args(&["carol", "dave"]);
        let applied = apply(&mut chan, &request(true, "+ooXX", &list), resolve);
        assert_eq!(
            applied.errors,
            [
                ErrorReply::UserNotInChannel("carol".into(), "#sampo".into()),
                ErrorReply::NoSuchNick("dave".into()),
                ErrorReply::UnknownMode('X'),
            ]
        );
    }

    #[test]
    fn at_most_four_parameters_are_taken() {
        let mut chan = channel();
        let list = args(&["bob", "bob", "alice", "alice", "bob"]);
        let applied = apply(&mut chan, &request(true, "+vvvvv", &list), resolve);
        assert_eq!(
            applied.changes.render().1.len(),
            2,
            "only real changes count, but only four are tried"
        );
    }
}
