use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Instant, sleep_until},
};

use crate::Report;

const RELOAD_DEBOUNCE: Duration = Duration::from_millis(200);
const RELOAD_MAX_DELAY: Duration = Duration::from_secs(2);

/// The live-reload revision: starts at 0 and counts content changes.
#[derive(Clone)]
pub(crate) struct ReloadState {
    revision: watch::Sender<u64>,
}

impl ReloadState {
    #[must_use]
    pub(crate) fn current(&self) -> u64 {
        *self.revision.borrow()
    }

    #[must_use]
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }
}

fn bump(revision: &watch::Sender<u64>) {
    revision.send_modify(|revision| *revision = revision.wrapping_add(1));
}

pub(crate) struct LiveWatcher {
    _watcher: RecommendedWatcher,
    task: JoinHandle<()>,
}

impl Drop for LiveWatcher {
    fn drop(&mut self) {
        let Self { _watcher: _, task } = self;
        task.abort();
    }
}

/// Watches `root` and bumps the revision when visible files change.
pub(crate) fn start(root: &Path) -> Result<(LiveWatcher, ReloadState), Report> {
    let root = root.to_owned();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = event_tx.send(event);
    })?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| {
            io::Error::other(format!(
                "could not watch {} for changes: {error}",
                root.display()
            ))
        })?;

    let (revision, _) = watch::channel(0);
    let state = ReloadState {
        revision: revision.clone(),
    };
    let task = tokio::spawn(run_debouncer(event_rx, revision, root));
    Ok((
        LiveWatcher {
            _watcher: watcher,
            task,
        },
        state,
    ))
}

async fn run_debouncer(
    mut events: mpsc::UnboundedReceiver<notify::Result<Event>>,
    revision: watch::Sender<u64>,
    root: PathBuf,
) {
    while let Some(event) = events.recv().await {
        if !should_reload(event, &root) {
            continue;
        }

        let maximum_deadline = Instant::now() + RELOAD_MAX_DELAY;
        let deadline = sleep_until(Instant::now() + RELOAD_DEBOUNCE);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                () = &mut deadline => {
                    bump(&revision);
                    break;
                }
                event = events.recv() => {
                    let Some(event) = event else {
                        bump(&revision);
                        tracing::warn!(
                            "filesystem watcher stopped; live reload is no longer active"
                        );
                        return;
                    };
                    if should_reload(event, &root) {
                        deadline
                            .as_mut()
                            .reset((Instant::now() + RELOAD_DEBOUNCE).min(maximum_deadline));
                    }
                }
            }
        }
    }
    tracing::warn!("filesystem watcher stopped; live reload is no longer active");
}

fn should_reload(event: notify::Result<Event>, root: &Path) -> bool {
    let event = match event {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(%error, "filesystem watcher reported an error");
            return false;
        }
    };
    match event.kind {
        EventKind::Access(_) => false,
        EventKind::Any
        | EventKind::Create(_)
        | EventKind::Modify(_)
        | EventKind::Remove(_)
        | EventKind::Other => paths_trigger_reload(&event.paths, root),
    }
}

fn paths_trigger_reload(paths: &[PathBuf], root: &Path) -> bool {
    paths.is_empty()
        || paths
            .iter()
            .any(|path| is_visible_path(&absolute_event_path(path, root), root))
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
    use std::path::Path;

    use notify::{
        Event, EventKind,
        event::{AccessKind, ModifyKind},
    };

    use crate::share::live::should_reload;

    #[test]
    fn filters_hidden_and_read_events() {
        let root = Path::new("/site");
        let modify = |path| Ok(Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path));
        assert!(!should_reload(modify(root.join(".git/index")), root));
        assert!(should_reload(modify(root.join("index.html")), root));
        assert!(!should_reload(
            Ok(Event::new(EventKind::Access(AccessKind::Read)).add_path(root.join("index.html"))),
            root,
        ));
        assert!(should_reload(Ok(Event::new(EventKind::Any)), root));
    }
}
