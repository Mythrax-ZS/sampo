//! Line framing: cutting a byte stream into messages.

use std::io;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use super::MAX_LINE;

/// Reads lines from a byte stream, the way IRC servers always have.
///
/// A line may end in CR-LF, a bare LF or a bare CR, and empty lines are
/// skipped. A line longer than the limit is cut short and the excess thrown
/// away. NUL bytes, which RFC 1459 forbids, are dropped. Text that is not
/// valid UTF-8 is read as Latin-1, which is what the 8-bit clients of the
/// 1990s nearly always meant.
#[derive(Debug)]
pub struct LineReader<R> {
    inner: BufReader<R>,
    line: Vec<u8>,
    limit: usize,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    /// Wraps a stream, allowing lines of 510 bytes: 512 with the CR-LF.
    pub fn new(inner: R) -> Self {
        Self::with_limit(inner, MAX_LINE - 2)
    }

    /// Wraps a stream, allowing lines of up to `limit` bytes.
    pub fn with_limit(inner: R, limit: usize) -> Self {
        LineReader {
            inner: BufReader::new(inner),
            line: Vec::with_capacity(limit.min(1024)),
            limit,
        }
    }

    /// Reads the next line, without its terminator.
    ///
    /// Returns `Ok(None)` once the stream has ended. The future is cancel
    /// safe: a line half read when it is dropped is finished by the next
    /// call.
    pub async fn next_line(&mut self) -> io::Result<Option<String>> {
        loop {
            let buf = self.inner.fill_buf().await?;
            if buf.is_empty() {
                // The stream has ended. A last line without its terminator
                // is still a line.
                return Ok((!self.line.is_empty()).then(|| self.take()));
            }
            let end = buf.iter().position(|&b| b == b'\n' || b == b'\r');
            let chunk = &buf[..end.unwrap_or(buf.len())];
            let room = self.limit.saturating_sub(self.line.len());
            self.line.extend(chunk.iter().copied().filter(|&b| b != 0).take(room));
            let used = chunk.len() + usize::from(end.is_some());
            self.inner.consume(used);
            if end.is_some() && !self.line.is_empty() {
                return Ok(Some(self.take()));
            }
        }
    }

    fn take(&mut self) -> String {
        decode(std::mem::take(&mut self.line))
    }
}

/// Decodes a line: as UTF-8 if it is valid, as Latin-1 if it is not.
pub fn decode(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => {
            let error = e.utf8_error();
            let bytes = e.into_bytes();
            if error.error_len().is_none() {
                // Good UTF-8, but for a last character cut in half when the
                // line was cut short.
                String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned()
            } else {
                bytes.into_iter().map(char::from).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn lines(input: &[u8], limit: usize) -> Vec<String> {
        let mut reader = LineReader::with_limit(input, limit);
        let mut out = Vec::new();
        while let Some(line) = reader.next_line().await.unwrap() {
            out.push(line);
        }
        out
    }

    #[tokio::test]
    async fn splits_on_any_line_ending() {
        let input = b"NICK alice\r\nUSER a 0 * :A\nPING x\rQUIT";
        assert_eq!(lines(input, 510).await, ["NICK alice", "USER a 0 * :A", "PING x", "QUIT"]);
    }

    #[tokio::test]
    async fn skips_empty_lines_and_nul_bytes() {
        assert_eq!(lines(b"\r\n\r\nPI\0NG\r\n\n", 510).await, ["PING"]);
    }

    #[tokio::test]
    async fn cuts_overlong_lines() {
        let mut input = b"PRIVMSG #a :".to_vec();
        input.extend([b'x'; 600]);
        input.extend(b"\r\nQUIT\r\n");
        let got = lines(&input, 510).await;
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].len(), 510);
        assert_eq!(got[1], "QUIT");
    }

    #[tokio::test]
    async fn reads_latin1_when_it_is_not_utf8() {
        assert_eq!(lines(b"PRIVMSG #a :p\xe4iv\xe4\r\n", 510).await, ["PRIVMSG #a :päivä"]);
        assert_eq!(lines("PRIVMSG #a :päivä\r\n".as_bytes(), 510).await, ["PRIVMSG #a :päivä"]);
    }

    #[tokio::test]
    async fn drops_a_character_cut_in_half() {
        // "äö" is four bytes; a limit of three cuts the ö in two.
        assert_eq!(lines("äö\r\n".as_bytes(), 3).await, ["ä"]);
    }
}
