//! Numeric replies.
//!
//! A server answers commands with three-digit codes: 001 to 099 for the
//! welcome, 200 to 399 for replies, and 400 to 599 for errors. The names
//! are those of RFC 1459 §6, misspellings and all, plus the few later
//! numerics that every server of the 1990s sent anyway.

use std::fmt;

/// A three-digit numeric reply code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Numeric(pub u16);

impl Numeric {
    /// Reads a numeric from a command word such as `"433"`.
    pub fn parse(command: &str) -> Option<Numeric> {
        if command.len() == 3 && command.bytes().all(|b| b.is_ascii_digit()) {
            command.parse().ok().map(Numeric)
        } else {
            None
        }
    }

    /// Whether this is an error reply, 400 to 599.
    pub fn is_error(self) -> bool {
        (400..600).contains(&self.0)
    }
}

impl fmt::Display for Numeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03}", self.0)
    }
}

// The welcome.

/// `:Welcome to the Internet Relay Network <nick>!<user>@<host>`
pub const RPL_WELCOME: Numeric = Numeric(1);
/// `:Your host is <servername>, running version <version>`
pub const RPL_YOURHOST: Numeric = Numeric(2);
/// `:This server was created <date>`
pub const RPL_CREATED: Numeric = Numeric(3);
/// `<servername> <version> <user modes> <channel modes>`
pub const RPL_MYINFO: Numeric = Numeric(4);
/// `<token>... :are supported by this server`
pub const RPL_ISUPPORT: Numeric = Numeric(5);

// Command replies.

/// `<user mode string>`
pub const RPL_UMODEIS: Numeric = Numeric(221);
/// `:There are <users> users and <invisible> invisible on <servers> servers`
pub const RPL_LUSERCLIENT: Numeric = Numeric(251);
/// `<count> :operator(s) online`
pub const RPL_LUSEROP: Numeric = Numeric(252);
/// `<count> :unknown connection(s)`
pub const RPL_LUSERUNKNOWN: Numeric = Numeric(253);
/// `<count> :channels formed`
pub const RPL_LUSERCHANNELS: Numeric = Numeric(254);
/// `:I have <clients> clients and <servers> servers`
pub const RPL_LUSERME: Numeric = Numeric(255);
/// `<nick> :<away message>`
pub const RPL_AWAY: Numeric = Numeric(301);
/// `:[<reply>{<space><reply>}]`
pub const RPL_USERHOST: Numeric = Numeric(302);
/// `:[<nick> {<space><nick>}]`
pub const RPL_ISON: Numeric = Numeric(303);
/// `:You are no longer marked as being away`
pub const RPL_UNAWAY: Numeric = Numeric(305);
/// `:You have been marked as being away`
pub const RPL_NOWAWAY: Numeric = Numeric(306);
/// `<nick> <user> <host> * :<real name>`
pub const RPL_WHOISUSER: Numeric = Numeric(311);
/// `<nick> <server> :<server info>`
pub const RPL_WHOISSERVER: Numeric = Numeric(312);
/// `<nick> :is an IRC operator`
pub const RPL_WHOISOPERATOR: Numeric = Numeric(313);
/// `<name> :End of /WHO list.`
pub const RPL_ENDOFWHO: Numeric = Numeric(315);
/// `<nick> <seconds> <signon> :seconds idle, signon time`
pub const RPL_WHOISIDLE: Numeric = Numeric(317);
/// `<nick> :End of /WHOIS list.`
pub const RPL_ENDOFWHOIS: Numeric = Numeric(318);
/// `<nick> :{[@|+]<channel><space>}`
pub const RPL_WHOISCHANNELS: Numeric = Numeric(319);
/// `Channel :Users  Name`
pub const RPL_LISTSTART: Numeric = Numeric(321);
/// `<channel> <# visible> :<topic>`
pub const RPL_LIST: Numeric = Numeric(322);
/// `:End of /LIST`
pub const RPL_LISTEND: Numeric = Numeric(323);
/// `<channel> <mode> <mode params>`
pub const RPL_CHANNELMODEIS: Numeric = Numeric(324);
/// `<channel> <creation time>`
pub const RPL_CREATIONTIME: Numeric = Numeric(329);
/// `<channel> :No topic is set`
pub const RPL_NOTOPIC: Numeric = Numeric(331);
/// `<channel> :<topic>`
pub const RPL_TOPIC: Numeric = Numeric(332);
/// `<channel> <set by> <set at>`
pub const RPL_TOPICWHOTIME: Numeric = Numeric(333);
/// `<nick> <channel>`
pub const RPL_INVITING: Numeric = Numeric(341);
/// `<version>.<debuglevel> <server> :<comments>`
pub const RPL_VERSION: Numeric = Numeric(351);
/// `<channel> <user> <host> <server> <nick> <H|G>[*][@|+] :<hopcount> <real name>`
pub const RPL_WHOREPLY: Numeric = Numeric(352);
/// `<=|*|@> <channel> :[[@|+]<nick> [[@|+]<nick> [...]]]`
pub const RPL_NAMREPLY: Numeric = Numeric(353);
/// `<channel> :End of /NAMES list.`
pub const RPL_ENDOFNAMES: Numeric = Numeric(366);
/// `<channel> <ban mask> <set by> <set at>`
pub const RPL_BANLIST: Numeric = Numeric(367);
/// `<channel> :End of channel ban list`
pub const RPL_ENDOFBANLIST: Numeric = Numeric(368);
/// `:<string>`
pub const RPL_INFO: Numeric = Numeric(371);
/// `:- <text>`
pub const RPL_MOTD: Numeric = Numeric(372);
/// `:End of /INFO list.`
pub const RPL_ENDOFINFO: Numeric = Numeric(374);
/// `:- <server> Message of the day - `
pub const RPL_MOTDSTART: Numeric = Numeric(375);
/// `:End of /MOTD command.`
pub const RPL_ENDOFMOTD: Numeric = Numeric(376);
/// `:You are now an IRC operator`
pub const RPL_YOUREOPER: Numeric = Numeric(381);
/// `<server> :<string showing server's local time>`
pub const RPL_TIME: Numeric = Numeric(391);

// Error replies.

/// `<nickname> :No such nick/channel`
pub const ERR_NOSUCHNICK: Numeric = Numeric(401);
/// `<channel name> :No such channel`
pub const ERR_NOSUCHCHANNEL: Numeric = Numeric(403);
/// `<channel name> :Cannot send to channel`
pub const ERR_CANNOTSENDTOCHAN: Numeric = Numeric(404);
/// `<channel name> :You have joined too many channels`
pub const ERR_TOOMANYCHANNELS: Numeric = Numeric(405);
/// `:No origin specified`
pub const ERR_NOORIGIN: Numeric = Numeric(409);
/// `:No recipient given (<command>)`
pub const ERR_NORECIPIENT: Numeric = Numeric(411);
/// `:No text to send`
pub const ERR_NOTEXTTOSEND: Numeric = Numeric(412);
/// `<command> :Unknown command`
pub const ERR_UNKNOWNCOMMAND: Numeric = Numeric(421);
/// `:MOTD File is missing`
pub const ERR_NOMOTD: Numeric = Numeric(422);
/// `:No nickname given`
pub const ERR_NONICKNAMEGIVEN: Numeric = Numeric(431);
/// `<nick> :Erroneus nickname`
pub const ERR_ERRONEUSNICKNAME: Numeric = Numeric(432);
/// `<nick> :Nickname is already in use`
pub const ERR_NICKNAMEINUSE: Numeric = Numeric(433);
/// `<nick> <channel> :They aren't on that channel`
pub const ERR_USERNOTINCHANNEL: Numeric = Numeric(441);
/// `<channel> :You're not on that channel`
pub const ERR_NOTONCHANNEL: Numeric = Numeric(442);
/// `<user> <channel> :is already on channel`
pub const ERR_USERONCHANNEL: Numeric = Numeric(443);
/// `:You have not registered`
pub const ERR_NOTREGISTERED: Numeric = Numeric(451);
/// `<command> :Not enough parameters`
pub const ERR_NEEDMOREPARAMS: Numeric = Numeric(461);
/// `:You may not reregister`
pub const ERR_ALREADYREGISTRED: Numeric = Numeric(462);
/// `:Password incorrect`
pub const ERR_PASSWDMISMATCH: Numeric = Numeric(464);
/// `<channel> :Channel key already set`
pub const ERR_KEYSET: Numeric = Numeric(467);
/// `<channel> :Cannot join channel (+l)`
pub const ERR_CHANNELISFULL: Numeric = Numeric(471);
/// `<char> :is unknown mode char to me`
pub const ERR_UNKNOWNMODE: Numeric = Numeric(472);
/// `<channel> :Cannot join channel (+i)`
pub const ERR_INVITEONLYCHAN: Numeric = Numeric(473);
/// `<channel> :Cannot join channel (+b)`
pub const ERR_BANNEDFROMCHAN: Numeric = Numeric(474);
/// `<channel> :Cannot join channel (+k)`
pub const ERR_BADCHANNELKEY: Numeric = Numeric(475);
/// `<channel> <char> :Channel list is full`
pub const ERR_BANLISTFULL: Numeric = Numeric(478);
/// `:Permission Denied- You're not an IRC operator`
pub const ERR_NOPRIVILEGES: Numeric = Numeric(481);
/// `<channel> :You're not channel operator`
pub const ERR_CHANOPRIVSNEEDED: Numeric = Numeric(482);
/// `:No O-lines for your host`
pub const ERR_NOOPERHOST: Numeric = Numeric(491);
/// `:Unknown MODE flag`
pub const ERR_UMODEUNKNOWNFLAG: Numeric = Numeric(501);
/// `:Cant change mode for other users`
pub const ERR_USERSDONTMATCH: Numeric = Numeric(502);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_three_digits() {
        assert_eq!(RPL_WELCOME.to_string(), "001");
        assert_eq!(ERR_NICKNAMEINUSE.to_string(), "433");
    }

    #[test]
    fn reads_only_three_digits() {
        assert_eq!(Numeric::parse("001"), Some(RPL_WELCOME));
        assert_eq!(Numeric::parse("1"), None);
        assert_eq!(Numeric::parse("0001"), None);
        assert_eq!(Numeric::parse("PRIVMSG"), None);
        assert!(ERR_NOSUCHNICK.is_error());
        assert!(!RPL_TOPIC.is_error());
    }
}
