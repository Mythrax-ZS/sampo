//! What can go wrong: for the operator, and for the user.

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::net::SocketAddr;

use crate::proto::Numeric;
use crate::proto::numeric::*;

/// A failure that keeps the server from starting.
#[derive(Debug)]
pub enum Error {
    /// A listening socket could not be opened.
    Bind {
        /// The address that was wanted.
        addr: SocketAddr,
        /// Why it could not be had.
        source: io::Error,
    },
    /// The configuration does not describe a server that can run.
    Config(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Bind { addr, source } => write!(f, "cannot listen on {addr}: {source}"),
            Error::Config(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Bind { source, .. } => Some(source),
            Error::Config(_) => None,
        }
    }
}

/// A command refused: one variant for each error reply of RFC 1459 §6.1
/// that this server sends, carrying the names the reply mentions.
///
/// Command handlers return these, and the dispatcher turns them into
/// numerics for the client that sent the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorReply {
    /// 401: no one, and no channel, by that name.
    NoSuchNick(String),
    /// 403: no channel by that name.
    NoSuchChannel(String),
    /// 404: the channel's modes forbid the message.
    CannotSendToChan(String),
    /// 405: the client is already in as many channels as allowed.
    TooManyChannels(String),
    /// 409: PING without a token.
    NoOrigin,
    /// 411: PRIVMSG or NOTICE without a target.
    NoRecipient(&'static str),
    /// 412: PRIVMSG or NOTICE without text.
    NoTextToSend,
    /// 421: the command is not one this server knows.
    UnknownCommand(String),
    /// 422: there is no message of the day.
    NoMotd,
    /// 431: NICK, or WHOIS, without a nickname.
    NoNicknameGiven,
    /// 432: not a valid nickname.
    ErroneousNickname(String),
    /// 433: someone else has that nickname.
    NicknameInUse(String),
    /// 441: the named user is not in the named channel.
    UserNotInChannel(String, String),
    /// 442: the sender is not in that channel.
    NotOnChannel(String),
    /// 443: the named user is already in the named channel.
    UserOnChannel(String, String),
    /// 451: the command needs registration first.
    NotRegistered,
    /// 461: the command needs more parameters.
    NeedMoreParams(&'static str),
    /// 462: registration happens once.
    AlreadyRegistered,
    /// 464: the wrong password, for PASS or OPER.
    PasswdMismatch,
    /// 467: the channel already has a key.
    KeySet(String),
    /// 471: the channel is full.
    ChannelIsFull(String),
    /// 472: a channel mode this server does not know.
    UnknownMode(char),
    /// 473: the channel is invite-only.
    InviteOnlyChan(String),
    /// 474: the sender is banned from the channel.
    BannedFromChan(String),
    /// 475: the wrong channel key.
    BadChannelKey(String),
    /// 478: the channel's ban list is full.
    BanListFull(String),
    /// 481: the command is for IRC operators.
    NoPrivileges,
    /// 482: the command is for channel operators.
    ChanOpPrivsNeeded(String),
    /// 491: no operators are configured.
    NoOperHost,
    /// 501: a user mode this server does not know.
    UmodeUnknownFlag,
    /// 502: user modes are one's own business.
    UsersDontMatch,
}

impl ErrorReply {
    /// The numeric, the parameters that come before the text, and the
    /// text, worded as the RFC words it (and spelled so, too).
    pub fn parts(&self) -> (Numeric, Vec<String>, Cow<'static, str>) {
        use ErrorReply::*;
        let one = |name: &String| vec![name.clone()];
        let (numeric, params, text): (_, _, Cow<'static, str>) = match self {
            NoSuchNick(nick) => (ERR_NOSUCHNICK, one(nick), "No such nick/channel".into()),
            NoSuchChannel(chan) => (ERR_NOSUCHCHANNEL, one(chan), "No such channel".into()),
            CannotSendToChan(chan) => {
                (ERR_CANNOTSENDTOCHAN, one(chan), "Cannot send to channel".into())
            }
            TooManyChannels(chan) => {
                (ERR_TOOMANYCHANNELS, one(chan), "You have joined too many channels".into())
            }
            NoOrigin => (ERR_NOORIGIN, vec![], "No origin specified".into()),
            NoRecipient(command) => {
                (ERR_NORECIPIENT, vec![], format!("No recipient given ({command})").into())
            }
            NoTextToSend => (ERR_NOTEXTTOSEND, vec![], "No text to send".into()),
            UnknownCommand(command) => (ERR_UNKNOWNCOMMAND, one(command), "Unknown command".into()),
            NoMotd => (ERR_NOMOTD, vec![], "MOTD File is missing".into()),
            NoNicknameGiven => (ERR_NONICKNAMEGIVEN, vec![], "No nickname given".into()),
            // Sic: RFC 1459 spells it so, and so did ircd.
            ErroneousNickname(nick) => {
                (ERR_ERRONEUSNICKNAME, one(nick), "Erroneus nickname".into())
            }
            NicknameInUse(nick) => {
                (ERR_NICKNAMEINUSE, one(nick), "Nickname is already in use".into())
            }
            UserNotInChannel(nick, chan) => (
                ERR_USERNOTINCHANNEL,
                vec![nick.clone(), chan.clone()],
                "They aren't on that channel".into(),
            ),
            NotOnChannel(chan) => {
                (ERR_NOTONCHANNEL, one(chan), "You're not on that channel".into())
            }
            UserOnChannel(nick, chan) => (
                ERR_USERONCHANNEL,
                vec![nick.clone(), chan.clone()],
                "is already on channel".into(),
            ),
            NotRegistered => (ERR_NOTREGISTERED, vec![], "You have not registered".into()),
            NeedMoreParams(command) => {
                (ERR_NEEDMOREPARAMS, vec![command.to_string()], "Not enough parameters".into())
            }
            AlreadyRegistered => (ERR_ALREADYREGISTRED, vec![], "You may not reregister".into()),
            PasswdMismatch => (ERR_PASSWDMISMATCH, vec![], "Password incorrect".into()),
            KeySet(chan) => (ERR_KEYSET, one(chan), "Channel key already set".into()),
            ChannelIsFull(chan) => {
                (ERR_CHANNELISFULL, one(chan), "Cannot join channel (+l)".into())
            }
            UnknownMode(mode) => {
                (ERR_UNKNOWNMODE, vec![mode.to_string()], "is unknown mode char to me".into())
            }
            InviteOnlyChan(chan) => {
                (ERR_INVITEONLYCHAN, one(chan), "Cannot join channel (+i)".into())
            }
            BannedFromChan(chan) => {
                (ERR_BANNEDFROMCHAN, one(chan), "Cannot join channel (+b)".into())
            }
            BadChannelKey(chan) => {
                (ERR_BADCHANNELKEY, one(chan), "Cannot join channel (+k)".into())
            }
            BanListFull(chan) => {
                (ERR_BANLISTFULL, vec![chan.clone(), "b".into()], "Channel list is full".into())
            }
            NoPrivileges => {
                (ERR_NOPRIVILEGES, vec![], "Permission Denied- You're not an IRC operator".into())
            }
            ChanOpPrivsNeeded(chan) => {
                (ERR_CHANOPRIVSNEEDED, one(chan), "You're not channel operator".into())
            }
            NoOperHost => (ERR_NOOPERHOST, vec![], "No O-lines for your host".into()),
            UmodeUnknownFlag => (ERR_UMODEUNKNOWNFLAG, vec![], "Unknown MODE flag".into()),
            // Sic, again.
            UsersDontMatch => {
                (ERR_USERSDONTMATCH, vec![], "Cant change mode for other users".into())
            }
        };
        (numeric, params, text)
    }
}

impl fmt::Display for ErrorReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (numeric, params, text) = self.parts();
        write!(f, "{numeric}")?;
        for param in params {
            write!(f, " {param}")?;
        }
        write!(f, " :{text}")
    }
}

impl std::error::Error for ErrorReply {}
