//! The `command` target: runs the argv given after `--` once per batch,
//! the batch as NDJSON on its stdin; exit status 0 means delivered. A run
//! that outlasts its timeout is killed and counts as failed.

use std::{ffi::OsString, io::ErrorKind, process::Stdio, time::Duration};

use tokio::{io::AsyncWriteExt, process::Command};

use crate::{core::queue::DeliveryResult, protocol::comment::Comment};

/// Delivers `batch` to `argv`; the string says what went wrong, if
/// anything. A command that cannot be started at all is `Gone`.
pub(crate) async fn deliver(
    argv: &[OsString],
    timeout: Duration,
    batch: &[Comment],
) -> (DeliveryResult, String) {
    let Some((program, args)) = argv.split_first() else {
        return (DeliveryResult::Gone, "no command".to_owned());
    };
    let name = program.to_string_lossy();
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let gone = matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::PermissionDenied
            );
            let result = if gone {
                DeliveryResult::Gone
            } else {
                DeliveryResult::Error
            };
            return (result, format!("could not run {name}: {error}"));
        }
    };
    let mut input = Vec::new();
    for comment in batch {
        // Serialising plain strings and ids cannot fail.
        if let Ok(line) = serde_json::to_vec(comment) {
            input.extend_from_slice(&line);
            input.push(b'\n');
        }
    }
    let stdin = child.stdin.take();
    let run = async {
        if let Some(mut stdin) = stdin {
            // A command that exits without reading is judged by its status.
            let _ = stdin.write_all(&input).await;
            drop(stdin);
        }
        child.wait().await
    };
    let status = tokio::time::timeout(timeout, run).await;
    match status {
        Ok(Ok(status)) if status.success() => (DeliveryResult::Ok, String::new()),
        Ok(Ok(status)) => (
            DeliveryResult::Error,
            format!("{name} exited with {status}"),
        ),
        Ok(Err(error)) => (DeliveryResult::Error, format!("{name}: {error}")),
        Err(_) => {
            let _ = child.kill().await;
            (
                DeliveryResult::Error,
                format!(
                    "{name} took longer than {}s and was killed",
                    timeout.as_secs()
                ),
            )
        }
    }
}
