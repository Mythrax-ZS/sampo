//! Upper and lower case, the Scandinavian way.
//!
//! > Because of IRC's scandanavian origin, the characters {}| are
//! > considered to be the lower case equivalents of the characters []\,
//! > respectively. This is a critical issue when determining the
//! > equivalence of two nicknames.
//! >
//! > RFC 1459, §2.2 (the spelling is theirs)
//!
//! IRC was born at the University of Oulu in 1988, on terminals that spoke
//! the Finnish and Swedish dialect of 7-bit ASCII. On those screens the
//! bytes for `[\]` were the letters `ÄÖÅ` and the bytes for `{|}` were
//! `äöå`, so of course one set was the capital form of the other. Servers
//! went one step further and paired `^` with `~`, which were `Ü` and `ü`.
//! That mapping is what clients still call `CASEMAPPING=rfc1459`.

use std::fmt;

/// Folds one character to lower case under RFC 1459 rules.
///
/// ASCII 65 to 94 (`A`–`Z` and `[\]^`) become 97 to 126 (`a`–`z` and
/// `{|}~`). Everything else is left as it is.
pub const fn fold(c: char) -> char {
    match c {
        'A'..='^' => (c as u8 + 32) as char,
        _ => c,
    }
}

/// Whether two names are the same name, as far as IRC is concerned.
pub fn equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.chars().map(fold).eq(b.chars().map(fold))
}

/// A nickname or channel name folded to its canonical form.
///
/// Two names collide exactly when their `Folded` forms are equal, which
/// makes this the key for every table of names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Folded(String);

impl Folded {
    /// Folds a name.
    pub fn new(name: &str) -> Folded {
        Folded(name.chars().map(fold).collect())
    }

    /// The folded text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Folded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Matches `text` against a glob, ignoring case: `*` matches any run of
/// characters, `?` any one. This is `match()` from the original ircd's
/// match.c, less its backslash escapes.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().map(fold).collect();
    let text: Vec<char> = text.chars().map(fold).collect();
    let (mut p, mut t) = (0, 0);
    // Where to resume if the current attempt fails: just past the last `*`,
    // and the text position that star has so far swallowed up to.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p + 1, t));
                p += 1;
            }
            Some(&c) if c == '?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((after, swallowed)) => {
                    p = after;
                    t = swallowed + 1;
                    star = Some((after, t));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_the_scandinavian_brackets() {
        assert_eq!(Folded::new("Ilmari[OULU]\\^").as_str(), "ilmari{oulu}|~");
        assert!(equal("Ilmari[]", "ILMARI{}"));
        assert!(!equal("alice", "alice_"));
    }

    #[test]
    fn folds_only_the_seven_bit_ghosts_of_the_letters() {
        // `[` is the old Ä, and folds; the real Ä is not RFC 1459's business.
        assert!(equal("[", "{"));
        assert!(!equal("Ä", "ä"));
    }

    #[test]
    fn leaves_everything_else_alone() {
        assert_eq!(fold('_'), '_');
        assert_eq!(fold('`'), '`');
        assert_eq!(fold('Ä'), 'Ä');
        assert_eq!(fold('5'), '5');
    }

    #[test]
    fn matches_wildcards() {
        assert!(wildcard("*", ""));
        assert!(wildcard("*!*@*", "alice!~alice@10.0.0.7"));
        assert!(wildcard("ALICE!*@10.0.0.?", "alice!~a@10.0.0.7"));
        assert!(wildcard("*!*@*.fi", "jarkko!jto@tolsun.oulu.fi"));
        assert!(wildcard("a*b*c", "aXXbYYbZZc"));
        assert!(wildcard("{x}*", "[X]yz"));
        assert!(!wildcard("*!*@*.fi", "jarkko!jto@tolsun.oulu.se"));
        assert!(!wildcard("a?c", "ac"));
        assert!(!wildcard("abc", "abcd"));
    }
}
