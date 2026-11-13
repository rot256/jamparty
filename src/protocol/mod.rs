//! The share <-> joiner protocol over iroh (ALPN `jampot/1`).
//!
//! - Control: one long-lived bidirectional stream opened by the joiner;
//!   newline-delimited JSON, one [`control::Envelope`] per line. The
//!   joiner's first line is its hello (secret and name); nothing else is
//!   read or served before the share admits it.
//! - Requests: HTTP/1.1 on joiner-opened streams (not modelled here).
//!
//! [`agent`] is the local runtime socket used by `jampot recv` and
//! `jampot send`.

pub(crate) mod agent;
pub(crate) mod comment;
pub(crate) mod control;
pub(crate) mod presence;

use std::{
    error::Error,
    fmt,
    io::{self, Write},
};

use serde::{Serialize, de::DeserializeOwned};
use zeroize::Zeroizing;

/// The iroh ALPN. It stays the same across protocol versions (each line
/// carries its own), so a share and a joiner of different ones still
/// connect and can tell each other which jampot they run.
pub(crate) const ALPN: &[u8] = b"jampot/1";

/// Maximum length of one control line, newline included, in either
/// direction. A maximal comment draft with every character escaped as
/// `\uXXXX` is about 110 KiB.
pub(crate) const MAX_LINE_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub(crate) enum FrameError {
    /// The line (or the encoded message) exceeds [`MAX_LINE_BYTES`].
    TooLong,
    Json(serde_json::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => write!(f, "control line longer than {MAX_LINE_BYTES} bytes"),
            Self::Json(error) => write!(f, "invalid control line: {error}"),
        }
    }
}

impl Error for FrameError {}

/// `message` as one line: compact JSON and a newline.
///
/// A line may carry the party secret (the hello), so it is measured first
/// and written into a buffer of exactly its size, wiped on drop: a buffer
/// that grew would leave the bytes it held behind, unwiped.
pub(crate) fn encode_line<T: Serialize>(message: &T) -> Result<Zeroizing<Vec<u8>>, FrameError> {
    let mut length = Length(0);
    serde_json::to_writer(&mut length, message).map_err(FrameError::Json)?;
    let length = length.0.saturating_add(1);
    if length > MAX_LINE_BYTES {
        return Err(FrameError::TooLong);
    }
    let mut line = Zeroizing::new(Vec::with_capacity(length));
    serde_json::to_writer(&mut *line, message).map_err(FrameError::Json)?;
    line.push(b'\n');
    Ok(line)
}

/// Counts the bytes written to it, keeping none.
struct Length(usize);

impl Write for Length {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Parses one line, with or without its trailing newline.
pub(crate) fn decode_line<T: DeserializeOwned>(line: &[u8]) -> Result<T, FrameError> {
    if line.len() > MAX_LINE_BYTES {
        return Err(FrameError::TooLong);
    }
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    serde_json::from_slice(body).map_err(FrameError::Json)
}

#[cfg(test)]
mod tests {
    use crate::protocol::{FrameError, MAX_LINE_BYTES, decode_line, encode_line};

    #[test]
    fn lines_round_trip_and_are_bounded() {
        let line = encode_line(&vec!["a", "b"]).unwrap();
        assert_eq!(line.as_slice(), b"[\"a\",\"b\"]\n");
        // Sized exactly, so it never reallocated (see encode_line).
        assert_eq!(line.capacity(), line.len());
        assert_eq!(decode_line::<Vec<String>>(&line).unwrap(), ["a", "b"]);
        assert_eq!(
            decode_line::<Vec<String>>(b"[]").unwrap(),
            Vec::<String>::new()
        );
        assert!(matches!(
            decode_line::<Vec<String>>(b"[]\n[]\n"),
            Err(FrameError::Json(_))
        ));
        assert!(matches!(
            encode_line(&"x".repeat(MAX_LINE_BYTES)),
            Err(FrameError::TooLong)
        ));
        let long = vec![b' '; MAX_LINE_BYTES + 1];
        assert!(matches!(
            decode_line::<Vec<String>>(&long),
            Err(FrameError::TooLong)
        ));
    }
}
