//! Control-stream framing on async streams: one bounded JSON line at a
//! time.

use rootcause::prelude::bail;
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
};
use zeroize::Zeroizing;

use crate::{
    Report,
    protocol::{MAX_LINE_BYTES, decode_line, encode_line},
};

/// A read line's buffer starts this large. The share reads the hello (the
/// line with the party secret) with [`read_hello`] instead; the joiner's
/// lines from the share carry no secret.
const LINE_CAPACITY: usize = 1024;

/// The longest hello the share reads (the line with the party secret).
pub(crate) const MAX_HELLO: usize = 1024;

/// Reads and parses one line; `None` at a clean end of stream. The line's
/// buffer is wiped once it is parsed.
pub(crate) async fn read<T, R>(reader: &mut R) -> Result<Option<T>, Report>
where
    T: DeserializeOwned,
    R: AsyncBufRead + Unpin,
{
    let mut line = Zeroizing::new(Vec::with_capacity(LINE_CAPACITY));
    let limit = u64::try_from(MAX_LINE_BYTES)?.saturating_add(1);
    let read = (&mut *reader)
        .take(limit)
        .read_until(b'\n', &mut line)
        .await?;
    if read == 0 {
        return Ok(None);
    }
    if !line.ends_with(b"\n") {
        bail!("control line too long or cut off");
    }
    Ok(Some(decode_line(&line)?))
}

/// Reads the first line of a control stream, the hello that carries the
/// party secret, without a buffered reader: straight into a fixed buffer
/// of [`MAX_HELLO`] bytes, wiped when it drops (also when the read is
/// cancelled or fails). A line with a backslash is refused before parsing:
/// an honest hello has nothing to escape (names and the secret's base32
/// do not), and an escape would make the parser unescape the secret into
/// a buffer of its own that nobody wipes; without one, the secret is only
/// ever borrowed from ours. Returns the parsed line (`None` at a clean end
/// of stream) and the bytes that came after it in the same read, which
/// belong to the next lines.
pub(crate) async fn read_hello<T, R>(reader: &mut R) -> Result<(Option<T>, Vec<u8>), Report>
where
    T: DeserializeOwned,
    R: AsyncRead + Unpin,
{
    let mut buffer = Zeroizing::new([0_u8; MAX_HELLO]);
    let mut filled = 0;
    loop {
        let Some(space) = buffer.get_mut(filled..).filter(|space| !space.is_empty()) else {
            bail!("no hello within {MAX_HELLO} bytes");
        };
        let read = reader.read(space).await?;
        if read == 0 {
            if filled == 0 {
                return Ok((None, Vec::new()));
            }
            bail!("the hello is cut off");
        }
        let start = filled;
        filled = filled.saturating_add(read);
        let fresh = buffer.get(start..filled).unwrap_or_default();
        let Some(offset) = fresh.iter().position(|byte| *byte == b'\n') else {
            continue;
        };
        let end = start.saturating_add(offset).saturating_add(1);
        let (line, rest) = buffer.split_at(end);
        let leftover = rest
            .get(..filled.saturating_sub(end))
            .unwrap_or_default()
            .to_vec();
        if line.contains(&b'\\') {
            bail!("the hello has an escape");
        }
        return Ok((Some(decode_line(line)?), leftover));
    }
}

/// Encodes and writes one line.
pub(crate) async fn write<T, W>(writer: &mut W, message: &T) -> Result<(), Report>
where
    T: Serialize,
    W: AsyncWrite + Unpin,
{
    writer.write_all(&encode_line(message)?).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use crate::{
        adapters::lines::{MAX_HELLO, read, read_hello, write},
        protocol::MAX_LINE_BYTES,
    };

    #[tokio::test]
    async fn hellos_are_bounded_and_unescaped() {
        let both: &[u8] = b"[\"a\"]\n[\"b\"]\n";
        let mut reader = both;
        let (first, rest) = read_hello::<Vec<String>, _>(&mut reader).await.unwrap();
        assert_eq!(first, Some(vec!["a".to_owned()]));
        assert_eq!(rest, b"[\"b\"]\n");
        let mut empty: &[u8] = b"";
        assert!(
            read_hello::<Vec<String>, _>(&mut empty)
                .await
                .unwrap()
                .0
                .is_none()
        );
        let mut escaped: &[u8] = b"[\"\\u0061\"]\n";
        assert!(read_hello::<Vec<String>, _>(&mut escaped).await.is_err());
        let long = [vec![b' '; MAX_HELLO], b"[]\n".to_vec()].concat();
        assert!(
            read_hello::<Vec<String>, _>(&mut long.as_slice())
                .await
                .is_err()
        );
        let fits = [vec![b' '; MAX_HELLO - 3], b"[]\n".to_vec()].concat();
        assert!(
            read_hello::<Vec<String>, _>(&mut fits.as_slice())
                .await
                .is_ok()
        );
        let mut cut: &[u8] = b"[";
        assert!(read_hello::<Vec<String>, _>(&mut cut).await.is_err());
    }

    #[tokio::test]
    async fn round_trip_and_limits() {
        let mut buffer = Vec::new();
        write(&mut buffer, &vec![1, 2]).await.unwrap();
        write(&mut buffer, &vec![3]).await.unwrap();
        let mut reader = BufReader::new(buffer.as_slice());
        assert_eq!(
            read::<Vec<u8>, _>(&mut reader).await.unwrap(),
            Some(vec![1, 2])
        );
        assert_eq!(
            read::<Vec<u8>, _>(&mut reader).await.unwrap(),
            Some(vec![3])
        );
        assert_eq!(read::<Vec<u8>, _>(&mut reader).await.unwrap(), None);

        let cut: &[u8] = b"[1";
        assert!(read::<Vec<u8>, _>(&mut BufReader::new(cut)).await.is_err());
        let long = vec![b' '; MAX_LINE_BYTES + 10];
        assert!(
            read::<Vec<u8>, _>(&mut BufReader::new(long.as_slice()))
                .await
                .is_err()
        );
    }
}
