//! The `command` target: runs the argv given after `--` once per batch,
//! the batch as NDJSON on its stdin; exit status 0 means delivered. A run
//! that outlasts its timeout is killed, with everything it started (it
//! runs in a process group of its own), and counts as failed.

use std::{ffi::OsString, io::ErrorKind, process::Stdio, time::Duration};

use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use tokio::{io::AsyncWriteExt, process::Command, time};

use crate::{core::queue::Attempt, protocol::comment::Comment};

/// Delivers `batch` to `argv`. A command that cannot be started at all is
/// gone.
pub(crate) async fn deliver(argv: &[OsString], timeout: Duration, batch: &[Comment]) -> Attempt {
    let Some((program, args)) = argv.split_first() else {
        return Attempt::Gone {
            why: "no command".to_owned(),
        };
    };
    let name = program.to_string_lossy();
    let mut input = Vec::new();
    for comment in batch {
        if let Err(error) = serde_json::to_writer(&mut input, comment) {
            return Attempt::Failed {
                why: format!("could not encode a comment: {error}"),
            };
        }
        input.push(b'\n');
    }
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let gone = matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::PermissionDenied
            );
            let why = format!("could not run {name}: {error}");
            return if gone {
                Attempt::Gone { why }
            } else {
                Attempt::Failed { why }
            };
        }
    };
    let group = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .map(Pid::from_raw);
    let stdin = child.stdin.take();
    let run = async {
        if let Some(mut stdin) = stdin {
            // A command that exits without reading is judged by its status.
            let _ = stdin.write_all(&input).await;
            drop(stdin);
        }
        child.wait().await
    };
    let status = time::timeout(timeout, run).await;
    match status {
        Ok(Ok(status)) if status.success() => Attempt::Delivered,
        Ok(Ok(status)) => Attempt::Failed {
            why: format!("{name} exited with {status}"),
        },
        Ok(Err(error)) => Attempt::Failed {
            why: format!("{name}: {error}"),
        },
        Err(_) => {
            if let Some(group) = group {
                let _ = killpg(group, Signal::SIGKILL);
            }
            let _ = child.kill().await;
            Attempt::Failed {
                why: format!(
                    "{name} took longer than {}s and was killed",
                    timeout.as_secs()
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
    use tokio::time;

    use crate::{adapters::command::deliver, core::queue::Attempt};

    #[tokio::test]
    async fn maximal_comments_fit_and_backlogs_reach_command_in_bounded_batches() {
        use crate::{
            core::{
                ids::{CommentId, EndpointKey},
                name::MAX_NAME,
                queue::{DeliveryQueue, DeliveryResult},
                rng::SeededRng,
            },
            protocol::{
                comment::{
                    Author, Comment, Draft, DraftFields, MAX_COMMENT, MAX_PAGE, MAX_SELECTED_TEXT,
                    MAX_SELECTOR,
                },
                line_size,
            },
            share::delivery::MAX_BATCH_BYTES,
        };

        // U+0000 takes the longest JSON escape (six bytes) and is valid
        // in every free-text field, including a nonblank comment.
        let draft = Draft::try_from(DraftFields {
            page: "\0".repeat(MAX_PAGE).try_into().unwrap(),
            selector: "\0".repeat(MAX_SELECTOR).try_into().unwrap(),
            selected_text: Some("\0".repeat(MAX_SELECTED_TEXT)),
            comment: "\0".repeat(MAX_COMMENT),
        })
        .unwrap();
        let comment = Comment::new(
            CommentId::generate(&SeededRng::new(1)),
            Author::new(
                "x".repeat(MAX_NAME).try_into().unwrap(),
                EndpointKey([0xff; 32]),
            ),
            draft,
        );
        let size = line_size(&comment).unwrap();
        assert!(size <= MAX_BATCH_BYTES, "maximal comment is {size} bytes");
        eprintln!("maximal comment NDJSON: {size} bytes");
        let now = jiff::Timestamp::from_second(1_800_000_000).unwrap();
        let mut queue = DeliveryQueue::new();
        for _ in 0..3 {
            queue.push(comment.clone()).unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("batch");
        let argv = [
            "sh".into(),
            "-c".into(),
            "cat > \"$0\"".into(),
            output.clone().into(),
        ];
        for _ in 0..3 {
            let batch = queue
                .start(now, MAX_BATCH_BYTES, |c| line_size(c).unwrap())
                .unwrap();
            assert_eq!(batch, vec![comment.clone()]);
            assert_eq!(
                deliver(&argv, Duration::from_secs(5), &batch).await,
                Attempt::Delivered
            );
            let bytes = fs::read(&output).unwrap();
            assert_eq!(bytes.len(), size);
            assert_eq!(bytes.last(), Some(&b'\n'));
            assert_eq!(serde_json::from_slice::<Comment>(&bytes).unwrap(), comment);
            queue.finish(DeliveryResult::Ok, now).unwrap();
        }
        assert!(queue.is_empty());
    }

    /// Whether `pid` still runs. A killed process whose parent died stays
    /// a zombie until init reaps it, and some inits (a container's node,
    /// say) never do; a zombie runs nothing, so it counts as gone.
    fn running(pid: i32) -> bool {
        if kill(Pid::from_raw(pid), None) == Err(Errno::ESRCH) {
            return false;
        }
        // `/proc/<pid>/stat` is `pid (comm) state ...`; comm may hold spaces
        // and parentheses, so the state follows the last `)`.
        let zombie = fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
        });
        !zombie
    }

    #[tokio::test]
    async fn a_timeout_kills_what_the_command_started() {
        let dir = tempfile::tempdir().unwrap();
        let pid = dir.path().join("pid");
        let argv = [
            "sh".into(),
            "-c".into(),
            r#"sleep 30 & echo $! > "$0"; wait"#.into(),
            pid.clone().into(),
        ];
        let attempt = deliver(&argv, Duration::from_millis(300), &[]).await;
        assert!(matches!(attempt, Attempt::Failed { .. }), "{attempt:?}");
        let pid: i32 = fs::read_to_string(&pid).unwrap().trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while running(pid) {
            assert!(Instant::now() < deadline, "the background sleep survived");
            time::sleep(Duration::from_millis(20)).await;
        }
    }
}
