//! Control-stream framing on async streams: one bounded JSON line at a
//! time.

use rootcause::prelude::bail;
use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    Report,
    protocol::{MAX_LINE_BYTES, decode_line, encode_line},
};

/// Reads and parses one line; `None` at a clean end of stream.
pub(crate) async fn read<T, R>(reader: &mut R) -> Result<Option<T>, Report>
where
    T: DeserializeOwned,
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
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
        adapters::lines::{read, write},
        protocol::MAX_LINE_BYTES,
    };

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
