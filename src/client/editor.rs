//! The input line: a small line editor with history and Tab completion,
//! after the one ircII drew at the foot of the screen.

use unicode_width::UnicodeWidthChar;

use crate::proto::Folded;

/// How many lines of history are kept.
const HISTORY: usize = 100;

/// The line being typed.
#[derive(Debug, Default)]
pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// How far back into the history Up has gone, if it has been pressed.
    recall: Option<usize>,
    /// The line that was being typed before Up was pressed.
    draft: Vec<char>,
    completion: Option<Completion>,
}

/// A completion in progress: Tab again offers the next match.
#[derive(Debug)]
struct Completion {
    start: usize,
    matches: Vec<String>,
    index: usize,
    suffix: &'static str,
}

impl Editor {
    pub fn new() -> Editor {
        Editor::default()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Types a character. Control characters are not typed.
    pub fn insert(&mut self, c: char) {
        self.completion = None;
        let c = if c == '\t' { ' ' } else { c };
        if !c.is_control() {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    pub fn insert_str(&mut self, text: &str) {
        text.chars().for_each(|c| self.insert(c));
    }

    pub fn backspace(&mut self) {
        self.completion = None;
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        self.completion = None;
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.completion = None;
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.completion = None;
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        self.completion = None;
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.completion = None;
        self.cursor = self.chars.len();
    }

    /// Erases everything before the cursor.
    pub fn kill_before(&mut self) {
        self.completion = None;
        self.chars.drain(..self.cursor);
        self.cursor = 0;
    }

    /// Erases everything from the cursor on.
    pub fn kill_after(&mut self) {
        self.completion = None;
        self.chars.truncate(self.cursor);
    }

    /// Erases the word before the cursor.
    pub fn kill_word(&mut self) {
        self.completion = None;
        let mut start = self.cursor;
        while start > 0 && self.chars[start - 1] == ' ' {
            start -= 1;
        }
        while start > 0 && self.chars[start - 1] != ' ' {
            start -= 1;
        }
        self.chars.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Takes the line as typed, and remembers it.
    pub fn submit(&mut self) -> String {
        self.completion = None;
        self.recall = None;
        self.cursor = 0;
        let line: String = self.chars.drain(..).collect();
        if !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        line
    }

    /// Brings back the line before, from history.
    pub fn older(&mut self) {
        self.completion = None;
        let index = match self.recall {
            None if self.history.is_empty() => return,
            None => {
                self.draft = std::mem::take(&mut self.chars);
                self.history.len() - 1
            }
            Some(0) => return,
            Some(index) => index - 1,
        };
        self.recall = Some(index);
        self.chars = self.history[index].chars().collect();
        self.cursor = self.chars.len();
    }

    /// Brings back the line after, and after the last, the draft.
    pub fn newer(&mut self) {
        self.completion = None;
        let Some(index) = self.recall else {
            return;
        };
        if index + 1 < self.history.len() {
            self.recall = Some(index + 1);
            self.chars = self.history[index + 1].chars().collect();
        } else {
            self.recall = None;
            self.chars = std::mem::take(&mut self.draft);
        }
        self.cursor = self.chars.len();
    }

    /// Completes the word before the cursor from `candidates`, ignoring
    /// case; pressed again, offers the next match. A nick completed at the
    /// start of the line gets a colon after it, as is the custom.
    pub fn complete(&mut self, candidates: &[String]) {
        if let Some(completion) = &mut self.completion {
            completion.index = (completion.index + 1) % completion.matches.len();
            let word = completion.matches[completion.index].clone();
            let (start, suffix) = (completion.start, completion.suffix);
            self.replace(start, &word, suffix);
            return;
        }
        let start =
            self.chars[..self.cursor].iter().rposition(|&c| c == ' ').map_or(0, |space| space + 1);
        let prefix: String = self.chars[start..self.cursor].iter().collect();
        if prefix.is_empty() {
            return;
        }
        let folded = Folded::new(&prefix);
        let mut matches: Vec<String> = candidates
            .iter()
            .filter(|c| Folded::new(c).as_str().starts_with(folded.as_str()))
            .cloned()
            .collect();
        matches.sort_by_cached_key(|m| Folded::new(m));
        matches.dedup();
        let Some(first) = matches.first().cloned() else {
            return;
        };
        let suffix = if start == 0 && !prefix.starts_with(['/', '#']) { ": " } else { " " };
        self.replace(start, &first, suffix);
        self.completion = Some(Completion { start, matches, index: 0, suffix });
    }

    /// Puts `word` and `suffix` in place of everything from `start` to the
    /// cursor.
    fn replace(&mut self, start: usize, word: &str, suffix: &str) {
        let new: Vec<char> = word.chars().chain(suffix.chars()).collect();
        let end = start + new.len();
        self.chars.splice(start..self.cursor, new);
        self.cursor = end;
    }

    /// The part of the line that fits in `width` columns, and the column of
    /// the cursor within it. The last column is left free for the cursor.
    pub fn view(&self, width: usize) -> (String, usize) {
        let width = width.max(2);
        let w = |c: &char| c.width().unwrap_or(0);
        let mut start = 0;
        let mut column: usize = self.chars[..self.cursor].iter().map(w).sum();
        while column >= width {
            column -= w(&self.chars[start]);
            start += 1;
        }
        let mut shown = String::new();
        let mut used = 0;
        for c in &self.chars[start..] {
            if used + w(c) >= width {
                break;
            }
            used += w(c);
            shown.push(*c);
        }
        (shown, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Editor {
        let mut editor = Editor::new();
        editor.insert_str(text);
        editor
    }

    fn text(editor: &Editor) -> String {
        editor.chars.iter().collect()
    }

    #[test]
    fn edits_like_a_shell() {
        let mut editor = typed("hello world");
        editor.kill_word();
        assert_eq!(editor.submit(), "hello ");

        let mut editor = typed("abc");
        editor.home();
        editor.delete();
        editor.end();
        editor.insert('!');
        assert_eq!(editor.submit(), "bc!");

        let mut editor = typed("one two");
        (0..3).for_each(|_| editor.left());
        editor.kill_before();
        assert_eq!(editor.submit(), "two");
    }

    #[test]
    fn remembers_history_and_the_draft() {
        let mut editor = Editor::new();
        for line in ["first", "second"] {
            editor.insert_str(line);
            editor.submit();
        }
        editor.insert_str("draft");
        editor.older();
        assert_eq!(text(&editor), "second");
        editor.older();
        editor.older();
        assert_eq!(text(&editor), "first");
        editor.newer();
        assert_eq!(text(&editor), "second");
        editor.newer();
        assert_eq!(text(&editor), "draft");
    }

    #[test]
    fn completes_nicks_and_cycles_through_them() {
        let nicks = ["alice", "Albert", "bob"].map(String::from);
        let mut editor = typed("al");
        editor.complete(&nicks);
        assert_eq!(text(&editor), "Albert: ");
        editor.complete(&nicks);
        assert_eq!(text(&editor), "alice: ");
        editor.complete(&nicks);
        assert_eq!(text(&editor), "Albert: ");

        let mut editor = typed("hi B");
        editor.complete(&nicks);
        assert_eq!(text(&editor), "hi bob ");
    }

    #[test]
    fn scrolls_to_keep_the_cursor_in_view() {
        let editor = typed("abcdefghij");
        assert_eq!(editor.view(20), ("abcdefghij".into(), 10));
        assert_eq!(editor.view(5), ("ghij".into(), 4));
    }
}
