//! Runs a nudge command with a plain-text notice and a bounded lifetime.

use std::{ffi::OsString, process::Stdio, time::Duration};

use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use tokio::{io::AsyncWriteExt, process::Command, time};

/// Kills descendants too when the nudge future is cancelled.
struct ProcessGroup(Option<Pid>);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(group) = self.0 {
            let _ = killpg(group, Signal::SIGKILL);
        }
    }
}

/// Sends a plain-text reminder to the command. It cannot deliver comments.
pub(crate) async fn nudge(
    argv: &[OsString],
    timeout: Duration,
    notice: &str,
) -> Result<(), String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("no nudge command".to_owned());
    };
    let name = program.to_string_lossy();
    let input = notice.as_bytes();
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Err(format!("could not run {name}: {error}")),
    };
    let group = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .map(Pid::from_raw);
    let mut cleanup = ProcessGroup(group);
    let stdin = child.stdin.take();
    let run = async {
        if let Some(mut stdin) = stdin {
            // A command that exits without reading is judged by its status.
            let _ = stdin.write_all(input).await;
            drop(stdin);
        }
        child.wait().await
    };
    let status = time::timeout(timeout, run).await;
    let result = match status {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!("{name} exited with {status}")),
        Ok(Err(error)) => Err(format!("{name}: {error}")),
        Err(_) => {
            drop(ProcessGroup(cleanup.0.take()));
            let _ = child.kill().await;
            Err(format!(
                "{name} took longer than {}s and was killed",
                timeout.as_secs()
            ))
        }
    };
    cleanup.0 = None;
    result
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
    use tokio::time;

    use crate::adapters::command::nudge;

    #[tokio::test]
    async fn notice_is_plain_text_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("notice");
        let notice = "jamparty: 2 comments waiting on test; run jamparty recv --share test\n";
        let argv = [
            "sh".into(),
            "-c".into(),
            "cat > \"$0\"".into(),
            output.clone().into(),
        ];
        nudge(&argv, Duration::from_secs(5), notice).await.unwrap();
        assert_eq!(fs::read_to_string(output).unwrap(), notice);
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

    #[tokio::test(start_paused = true)]
    async fn a_timeout_kills_what_the_command_started() {
        let dir = tempfile::tempdir().unwrap();
        let pid = dir.path().join("pid");
        let argv = [
            "sh".into(),
            "-c".into(),
            r#"sleep 30 & echo $! > "$0"; wait"#.into(),
            pid.clone().into(),
        ];
        let task =
            tokio::spawn(async move { nudge(&argv, Duration::from_secs(60), "notice\n").await });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fs::read_to_string(&pid).is_ok_and(|text| text.trim().parse::<i32>().is_ok()) {
            assert!(Instant::now() < deadline, "command did not start");
            tokio::task::yield_now().await;
        }
        time::advance(Duration::from_secs(60)).await;
        let attempt = task.await.unwrap();
        assert!(attempt.unwrap_err().contains("was killed"));
        let pid: i32 = fs::read_to_string(&pid).unwrap().trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while running(pid) {
            assert!(Instant::now() < deadline, "the background sleep survived");
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_kills_what_the_command_started() {
        let dir = tempfile::tempdir().unwrap();
        let pid = dir.path().join("pid");
        let argv = [
            "sh".into(),
            "-c".into(),
            r#"sleep 30 & echo $! > "$0"; wait"#.into(),
            pid.clone().into(),
        ];
        let task =
            tokio::spawn(async move { nudge(&argv, Duration::from_secs(60), "notice\n").await });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fs::read_to_string(&pid).is_ok_and(|text| text.trim().parse::<i32>().is_ok()) {
            assert!(Instant::now() < deadline, "command did not start");
            tokio::task::yield_now().await;
        }
        let descendant = fs::read_to_string(&pid).unwrap().trim().parse().unwrap();
        assert!(running(descendant));
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let pid: i32 = fs::read_to_string(&pid).unwrap().trim().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while running(pid) {
            assert!(Instant::now() < deadline, "the background sleep survived");
            tokio::task::yield_now().await;
        }
    }
}
