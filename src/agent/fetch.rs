//! Fetch attachment bytes from the user's running shares by content address.

use std::{io::Write, path::Path, time::Duration};

use rootcause::{
    option_ext::OptionExt,
    prelude::{ResultExt, bail},
};
use tempfile::NamedTempFile;
use tokio::{
    io::{AsyncReadExt, BufReader},
    net::UnixStream,
    time,
};

use crate::{
    Report,
    adapters::{lines, runtime},
    describe,
    protocol::{
        agent::{AgentReply, AgentRequest},
        control::Envelope,
        resource::{MAX_FILE_BYTES, ResourceId},
    },
};

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn resource(dir: &Path, id: ResourceId) -> Result<Vec<u8>, Report> {
    let mut shares = runtime::share_ids(dir)?;
    shares.sort_by_key(ToString::to_string);
    let mut failures = Vec::new();
    for share in shares {
        let result = time::timeout(FETCH_TIMEOUT, async {
            let stream = runtime::connect(dir, Some(&share)).await?;
            fetch_from(stream, id).await
        })
        .await
        .context("attachment fetch timed out")
        .map_err(Report::from)
        .and_then(|result| result);
        match result {
            Ok(Some(bytes)) => return Ok(bytes),
            Ok(None) => {}
            Err(error) => failures.push(format!("{share}: {}", describe(&error))),
        }
    }
    if failures.is_empty() {
        bail!("attachment {id} not found on any running share");
    }
    bail!(
        "attachment {id} not found on any running share; lookup failures: {}",
        failures.join("; ")
    );
}

async fn fetch_from(stream: UnixStream, id: ResourceId) -> Result<Option<Vec<u8>>, Report> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    lines::write(&mut write, &Envelope::from(AgentRequest::Fetch { id })).await?;
    let reply = lines::read::<Envelope<AgentReply>, _>(&mut read)
        .await?
        .context("the share stopped during fetch")?;
    match reply.message {
        AgentReply::Resource => {
            let mut bytes = Vec::new();
            read.take((MAX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() > MAX_FILE_BYTES {
                bail!("the share returned an attachment larger than 8 MiB");
            }
            verify(id, &bytes)?;
            Ok(Some(bytes))
        }
        AgentReply::ResourceMissing => Ok(None),
        AgentReply::Error { error } => bail!("{error}"),
        AgentReply::Comment { .. } | AgentReply::End | AgentReply::Sent => {
            bail!("unexpected reply from the share")
        }
    }
}

/// The destination is exclusively caller-supplied. Verify before opening
/// it, including when it designates stdout.
pub(crate) fn write_verified(
    id: ResourceId,
    bytes: &[u8],
    output: &Path,
    stdout: &mut impl Write,
) -> Result<(), Report> {
    verify(id, bytes)?;
    if output == Path::new("-") {
        stdout.write_all(bytes)?;
        stdout.flush()?;
    } else {
        // Written beside the destination and moved into place only once
        // complete, so a failed write leaves nothing at the destination.
        let directory = match output.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let mut file = NamedTempFile::new_in(directory)
            .context_with(|| format!("could not create a file in {}", directory.display()))?;
        file.write_all(bytes)
            .context_with(|| format!("could not write {}", output.display()))?;
        file.persist_noclobber(output)
            .map_err(|error| error.error)
            .context_with(|| format!("could not create {}", output.display()))?;
    }
    Ok(())
}

fn verify(id: ResourceId, bytes: &[u8]) -> Result<(), Report> {
    if ResourceId::digest(bytes) != id {
        bail!("attachment {id} failed SHA3-256 verification");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tokio::{
        io::{AsyncWriteExt, BufReader},
        net::UnixListener,
    };

    use crate::{
        adapters::{lines, runtime},
        agent::fetch::{resource, write_verified},
        core::ids::EndpointKey,
        protocol::{
            agent::{AgentReply, AgentRequest},
            control::Envelope,
            resource::ResourceId,
        },
    };

    #[test]
    fn verifies_before_writing_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resource");
        let mut stdout = Vec::new();
        let id = ResourceId::digest(b"expected");
        assert!(write_verified(id, b"wrong", &path, &mut stdout).is_err());
        assert!(!path.exists());
        assert!(write_verified(id, b"wrong", Path::new("-"), &mut stdout).is_err());
        assert!(stdout.is_empty());
        write_verified(id, b"expected", &path, &mut stdout).unwrap();
        assert!(write_verified(ResourceId::digest(b"new"), b"new", &path, &mut stdout).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"expected");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["resource"], "no temporary file is left behind");
        write_verified(id, b"expected", Path::new("-"), &mut stdout).unwrap();
        assert_eq!(stdout, b"expected");
    }

    #[tokio::test]
    async fn searches_past_socket_protocol_and_verification_failures() {
        let base = tempfile::tempdir().unwrap();
        let dir = base.path().join("run");
        let mut shares: Vec<_> = (1..=4).map(|key| EndpointKey([key; 32]).sid()).collect();
        shares.sort_by_key(ToString::to_string);
        let [offline, malformed, corrupt, healthy] = shares.as_slice() else {
            unreachable!()
        };
        let (malformed_listener, _malformed_socket) = runtime::listen(&dir, malformed).unwrap();
        let (corrupt_listener, _corrupt_socket) = runtime::listen(&dir, corrupt).unwrap();
        let (healthy_listener, _healthy_socket) = runtime::listen(&dir, healthy).unwrap();
        let offline_listener = UnixListener::bind(dir.join(format!("{offline}.sock"))).unwrap();
        drop(offline_listener);

        async fn serve(listener: UnixListener, reply: Option<&[u8]>) {
            let mut remaining = 2;
            while remaining > 0 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let Some(request) =
                    lines::read::<Envelope<AgentRequest>, _>(&mut BufReader::new(&mut stream))
                        .await
                        .unwrap()
                else {
                    // Starting another runtime socket probes existing listeners.
                    continue;
                };
                remaining -= 1;
                let AgentRequest::Fetch { id } = request.message else {
                    unreachable!()
                };
                match reply {
                    Some(bytes) if bytes == b"expected" && id != ResourceId::digest(bytes) => {
                        lines::write(&mut stream, &Envelope::from(AgentReply::ResourceMissing))
                            .await
                            .unwrap();
                    }
                    Some(bytes) => {
                        lines::write(&mut stream, &Envelope::from(AgentReply::Resource))
                            .await
                            .unwrap();
                        stream.write_all(bytes).await.unwrap();
                    }
                    None => stream.write_all(b"not JSON\n").await.unwrap(),
                }
            }
        }
        let tasks = [
            tokio::spawn(serve(malformed_listener, None)),
            tokio::spawn(serve(corrupt_listener, Some(b"wrong"))),
            tokio::spawn(serve(healthy_listener, Some(b"expected"))),
        ];
        assert_eq!(
            resource(&dir, ResourceId::digest(b"expected"))
                .await
                .unwrap(),
            b"expected"
        );
        let error = resource(&dir, ResourceId::digest(b"missing"))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("not found"), "{error}");
        for failed in [offline, malformed, corrupt] {
            assert!(error.contains(&failed.to_string()), "{error}");
        }
        assert!(!error.contains(&healthy.to_string()), "{error}");
        for task in tasks {
            task.await.unwrap();
        }
    }
}
