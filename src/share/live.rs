use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

use notify::{
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{AccessKind, AccessMode, CreateKind, ModifyKind},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior, interval_at, sleep_until},
};

use crate::{Report, protocol::control::Revision};

const RELOAD_DEBOUNCE: Duration =
    Duration::from_millis(if cfg!(target_os = "linux") { 200 } else { 750 });
const ROOT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const RELOAD_MAX_DELAY: Duration = Duration::from_secs(2);

/// The live-reload revision: starts at 0 and counts content changes.
#[derive(Clone)]
pub(crate) struct ReloadState {
    revision: watch::Sender<Revision>,
}

impl ReloadState {
    #[must_use]
    pub(crate) fn current(&self) -> Revision {
        *self.revision.borrow()
    }

    #[must_use]
    pub(crate) fn subscribe(&self) -> watch::Receiver<Revision> {
        self.revision.subscribe()
    }
}

fn bump(revision: &watch::Sender<Revision>) {
    revision.send_modify(|revision| *revision = revision.next());
}

pub(crate) struct LiveWatcher {
    task: JoinHandle<()>,
}

impl Drop for LiveWatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Watches visible entries under `root`, or just the selected file.
pub(crate) fn start(
    root: &Path,
    single: Option<PathBuf>,
) -> Result<(LiveWatcher, ReloadState), Report> {
    let root = root.to_owned();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = event_tx.send(event);
    })?;
    // Watch the containing directory so a rename over the file keeps the watch.
    let mode = if single.is_some() {
        RecursiveMode::NonRecursive
    } else {
        RecursiveMode::Recursive
    };
    let initial_identity = single.as_ref().and_then(|_| directory_identity(&root));
    watcher.watch(&root, mode).map_err(|error| {
        io::Error::other(format!(
            "could not watch {} for changes: {error}",
            root.display()
        ))
    })?;

    // Watching the parent of the watch root lets us reattach after the
    // watched directory is renamed, deleted, or recreated.
    let fallback = match (single.as_ref(), root.parent()) {
        (Some(_), Some(parent)) => match watcher.watch(parent, RecursiveMode::NonRecursive) {
            Ok(()) => None,
            Err(error) => {
                tracing::warn!(%error, "could not watch parent directory; polling for replacement");
                Some(initial_identity)
            }
        },
        (Some(_), None) | (None, _) => None,
    };

    let (revision, _) = watch::channel(Revision::default());
    let state = ReloadState {
        revision: revision.clone(),
    };
    let task = tokio::spawn(run_debouncer(
        watcher, event_rx, revision, root, single, fallback,
    ));
    Ok((LiveWatcher { task }, state))
}

async fn run_debouncer(
    mut watcher: RecommendedWatcher,
    mut events: mpsc::UnboundedReceiver<notify::Result<Event>>,
    revision: watch::Sender<Revision>,
    root: PathBuf,
    single: Option<PathBuf>,
    fallback: Option<Option<(u64, u64)>>,
) {
    let mut poll = fallback.map(|_| {
        let mut timer = interval_at(Instant::now() + ROOT_POLL_INTERVAL, ROOT_POLL_INTERVAL);
        timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
        timer
    });
    let mut watched_identity = fallback.flatten();
    let mut maximum_deadline = None;
    let deadline = sleep_until(Instant::now());
    tokio::pin!(deadline);

    loop {
        let reload = tokio::select! {
            biased;
            () = &mut deadline, if maximum_deadline.is_some() => {
                bump(&revision);
                maximum_deadline = None;
                continue;
            }
            _ = async {
                match &mut poll {
                    Some(timer) => timer.tick().await,
                    None => std::future::pending().await,
                }
            } => {
                let identity = directory_identity(&root);
                if identity == watched_identity {
                    continue;
                }
                let _ = watcher.unwatch(&root);
                watched_identity = None;
                if identity.is_some()
                    && let Err(error) = watcher.watch(&root, RecursiveMode::NonRecursive)
                {
                    tracing::warn!(%error, "could not reattach filesystem watcher");
                    continue;
                }
                watched_identity = identity;
                true
            }
            event = events.recv() => {
                let Some(event) = event else {
                    if maximum_deadline.is_some() {
                        bump(&revision);
                    }
                    tracing::warn!("filesystem watcher stopped; live reload is no longer active");
                    return;
                };
                if single.is_some()
                    && let Ok(event) = &event
                    && matches!(
                        event.kind,
                        EventKind::Any
                            | EventKind::Create(_)
                            | EventKind::Modify(ModifyKind::Any | ModifyKind::Name(_))
                            | EventKind::Remove(_)
                            | EventKind::Other
                    )
                    && event.paths.iter().any(|path| path == &root)
                {
                    if poll.is_some() {
                        // The replacement directory may reuse the previous
                        // inode; the next poll reattaches. Reload now, as the
                        // file may stay gone.
                        watched_identity = None;
                    } else {
                        // The backend may already have removed the old watch.
                        let _ = watcher.unwatch(&root);
                        if directory_identity(&root).is_some()
                            && let Err(error) = watcher.watch(&root, RecursiveMode::NonRecursive)
                        {
                            tracing::warn!(%error, "could not reattach filesystem watcher");
                        }
                    }
                    true
                } else {
                    should_reload(event, &root, single.as_deref())
                }
            }
        };
        if reload {
            let now = Instant::now();
            let maximum = *maximum_deadline.get_or_insert(now + RELOAD_MAX_DELAY);
            let quiet_until = now + RELOAD_DEBOUNCE;
            deadline.as_mut().reset(if single.is_some() {
                quiet_until
            } else {
                quiet_until.min(maximum)
            });
        }
    }
}

fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    metadata.is_dir().then(|| (metadata.dev(), metadata.ino()))
}

fn should_reload(event: notify::Result<Event>, root: &Path, single: Option<&Path>) -> bool {
    let event = match event {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(%error, "filesystem watcher reported an error");
            return false;
        }
    };
    match event.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => {
            paths_trigger_reload(&event.paths, root, single)
        }
        EventKind::Access(_) => false,
        // inotify delivers a close-write notification after the data events.
        // Waiting for it handles writers that pause longer than the debounce.
        EventKind::Modify(ModifyKind::Data(_)) if cfg!(target_os = "linux") => false,
        EventKind::Create(CreateKind::File) if cfg!(target_os = "linux") && single.is_some() => {
            false
        }
        EventKind::Any
        | EventKind::Create(_)
        | EventKind::Modify(_)
        | EventKind::Remove(_)
        | EventKind::Other => paths_trigger_reload(&event.paths, root, single),
    }
}

fn paths_trigger_reload(paths: &[PathBuf], root: &Path, single: Option<&Path>) -> bool {
    paths.is_empty()
        || paths.iter().any(|path| {
            let path = absolute_event_path(path, root);
            match single {
                Some(file) => path == file,
                None => is_visible_path(&path, root),
            }
        })
}

fn is_visible_path(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).map_or(true, |relative| {
        relative
            .components()
            .all(|component| !component.as_os_str().to_string_lossy().starts_with('.'))
    })
}

fn absolute_event_path(path: &Path, root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write as _, path::Path, time::Duration};

    use notify::{
        Event, EventKind,
        event::{AccessKind, ModifyKind},
    };
    use tokio::time::{sleep, timeout};

    use crate::{
        protocol::control::Revision,
        share::live::{should_reload, start},
    };

    #[tokio::test]
    async fn single_file_reload_follows_atomic_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("paper.pdf");
        fs::write(&file, "original").unwrap();
        let (_watcher, state) = start(&root, Some(file.clone())).unwrap();
        let mut revision = state.subscribe();
        let other = root.join("paper.tmp");
        fs::write(&other, "rebuilt").unwrap();
        assert!(
            timeout(Duration::from_secs(1), revision.changed())
                .await
                .is_err()
        );
        fs::rename(&other, &file).unwrap();
        timeout(Duration::from_secs(5), revision.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.current(), Revision::default().next());
    }

    #[tokio::test]
    async fn single_file_reload_follows_parent_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().canonicalize().unwrap();
        let root = parent.join("build");
        let file = root.join("paper.pdf");
        fs::create_dir(&root).unwrap();
        fs::write(&file, "original").unwrap();
        let (_watcher, state) = start(&root, Some(file.clone())).unwrap();
        let mut revision = state.subscribe();

        fs::rename(&root, parent.join("old")).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(&file, "replacement").unwrap();
        timeout(Duration::from_secs(5), revision.changed())
            .await
            .unwrap()
            .unwrap();

        fs::write(&file, "later write").unwrap();
        timeout(Duration::from_secs(5), revision.changed())
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn single_file_reload_with_unreadable_parent() {
        use std::os::unix::fs::PermissionsExt as _;

        if nix::unistd::geteuid().is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().canonicalize().unwrap().join("parent");
        let root = parent.join("build");
        let file = root.join("paper.pdf");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, "original").unwrap();
        let permissions = fs::metadata(&parent).unwrap().permissions();

        fs::set_permissions(&parent, fs::Permissions::from_mode(0o111)).unwrap();
        let started = start(&root, Some(file.clone()));
        let write = fs::write(&file, "first write");
        fs::set_permissions(&parent, permissions).unwrap();

        let (_watcher, state) = started.unwrap();
        let mut revision = state.subscribe();
        write.unwrap();
        timeout(Duration::from_secs(5), revision.changed())
            .await
            .unwrap()
            .unwrap();

        fs::rename(&root, parent.join("old")).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(&file, "replacement").unwrap();
        timeout(Duration::from_secs(5), revision.changed())
            .await
            .unwrap()
            .unwrap();

        fs::write(&file, "later write").unwrap();
        timeout(Duration::from_secs(5), revision.changed())
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn single_file_fallback_reattaches_after_inode_reuse() {
        use notify::{RecursiveMode, Watcher as _};
        use tokio::sync::{mpsc, watch};

        use crate::share::live::{directory_identity, run_debouncer};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("build");
        let file = root.join("paper.pdf");
        fs::create_dir(&root).unwrap();
        let (event_tx, events) = mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = event_tx.send(event);
        })
        .unwrap();
        watcher.watch(&root, RecursiveMode::NonRecursive).unwrap();

        fs::remove_dir(&root).unwrap();
        fs::create_dir(&root).unwrap();
        // Pretend the old watch was attached to the replacement's inode.
        let reused_identity = directory_identity(&root).unwrap();
        let (revision, mut changes) = watch::channel(Revision::default());
        let task = tokio::spawn(run_debouncer(
            watcher,
            events,
            revision,
            root,
            Some(file.clone()),
            Some(Some(reused_identity)),
        ));
        timeout(Duration::from_secs(5), changes.changed())
            .await
            .unwrap()
            .unwrap();

        fs::write(&file, "later write").unwrap();
        timeout(Duration::from_secs(5), changes.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*changes.borrow(), Revision::default().next().next());
        task.abort();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn single_file_fallback_reloads_while_the_directory_is_gone() {
        use notify::{RecursiveMode, Watcher as _};
        use tokio::sync::{mpsc, watch};

        use crate::share::live::{directory_identity, run_debouncer};

        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().canonicalize().unwrap();
        let root = parent.join("build");
        let file = root.join("paper.pdf");
        fs::create_dir(&root).unwrap();
        fs::write(&file, "original").unwrap();
        let (event_tx, events) = mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = event_tx.send(event);
        })
        .unwrap();
        watcher.watch(&root, RecursiveMode::NonRecursive).unwrap();
        let identity = directory_identity(&root).unwrap();
        let (revision, mut changes) = watch::channel(Revision::default());
        let task = tokio::spawn(run_debouncer(
            watcher,
            events,
            revision,
            root.clone(),
            Some(file.clone()),
            Some(Some(identity)),
        ));

        // Gone and staying gone: viewers must not keep the old file.
        fs::rename(&root, parent.join("old")).unwrap();
        timeout(Duration::from_secs(5), changes.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(!root.exists());

        fs::create_dir(&root).unwrap();
        fs::write(&file, "replacement").unwrap();
        timeout(Duration::from_secs(5), changes.changed())
            .await
            .unwrap()
            .unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn single_file_reload_waits_for_a_chunked_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("paper.pdf");
        fs::write(&file, "original").unwrap();
        let (_watcher, state) = start(&root, Some(file.clone())).unwrap();
        let mut revision = state.subscribe();
        let mut writer = fs::File::create(file).unwrap();
        for chunk in [b"first", b" next", b" last"] {
            writer.write_all(chunk).unwrap();
            sleep(Duration::from_millis(400)).await;
            assert_eq!(state.current(), Revision::default());
        }
        drop(writer);
        timeout(Duration::from_secs(5), revision.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.current(), Revision::default().next());
    }

    #[test]
    fn filters_hidden_and_read_events() {
        let root = Path::new("/site");
        let modify = |path| Ok(Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path));
        assert!(!should_reload(modify(root.join(".git/index")), root, None));
        assert!(should_reload(modify(root.join("index.html")), root, None));
        assert!(!should_reload(
            Ok(Event::new(EventKind::Access(AccessKind::Read)).add_path(root.join("index.html"))),
            root,
            None,
        ));
        assert!(should_reload(Ok(Event::new(EventKind::Any)), root, None));
    }
}
