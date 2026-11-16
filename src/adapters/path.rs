//! Selected iroh paths, with no peer addresses exposed to the page.
use std::{fmt, time::Duration};

use futures_util::StreamExt;
use iroh::{
    TransportAddr,
    endpoint::{Connection, PathList},
};
use serde::Serialize;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum PathKind {
    Direct,
    Relay,
    Mixed,
    Unknown,
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Direct => "direct",
            Self::Relay => "relay",
            Self::Mixed => "mixed",
            Self::Unknown => "unknown",
        })
    }
}

impl From<&PathList<'_>> for PathKind {
    fn from(paths: &PathList<'_>) -> Self {
        let mut direct = false;
        let mut relay = false;
        for path in paths.iter().filter(|path| path.is_selected()) {
            match path.remote_addr() {
                TransportAddr::Ip(_) => direct = true,
                TransportAddr::Relay(_) => relay = true,
                TransportAddr::Custom(_) => {}
                _ => {} // TransportAddr is non-exhaustive.
            }
        }
        Self::from((direct, relay))
    }
}

impl From<(bool, bool)> for PathKind {
    fn from((direct, relay): (bool, bool)) -> Self {
        match (direct, relay) {
            (true, false) => Self::Direct,
            (false, true) => Self::Relay,
            (true, true) => Self::Mixed,
            (false, false) => Self::Unknown,
        }
    }
}

struct Changes {
    said: PathKind,
    latest: PathKind,
    next: Instant,
}

impl Changes {
    fn new(initial: PathKind, now: Instant) -> Self {
        Self {
            said: initial,
            latest: initial,
            next: now,
        }
    }

    fn take(&mut self, now: Instant) -> Option<PathKind> {
        if self.latest == self.said || now < self.next {
            return None;
        }
        self.said = self.latest;
        self.next = now + Duration::from_secs(5);
        Some(self.said)
    }
}

/// Deduplicate snapshots and coalesce flapping to the latest state, at
/// most once per five seconds. Scoped by the caller's connection select:
/// dropping this future stops watching, with no spawned task to leak.
pub(crate) async fn changes(
    connection: &Connection,
    initial: PathKind,
    mut changed: impl FnMut(PathKind),
) {
    let mut stream = connection.paths_stream();
    let mut changes = Changes::new(initial, Instant::now());
    loop {
        tokio::select! {
            snapshot = stream.next() => {
                let Some(snapshot) = snapshot else { return };
                changes.latest = PathKind::from(&snapshot);
            }
            () = tokio::time::sleep_until(changes.next), if changes.latest != changes.said => {
                if let Some(path) = changes.take(Instant::now()) { changed(path); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::adapters::path::{Changes, PathKind};
    use std::time::Duration;
    use tokio::time::Instant;

    #[test]
    fn maps_selected_transports() {
        for (selected, expected) in [
            ((false, false), PathKind::Unknown),
            ((true, false), PathKind::Direct),
            ((false, true), PathKind::Relay),
            ((true, true), PathKind::Mixed),
        ] {
            assert_eq!(PathKind::from(selected), expected);
            assert_eq!(
                serde_json::to_string(&expected).unwrap(),
                format!("\"{expected}\"")
            );
        }
    }

    #[test]
    fn deduplicates_and_coalesces_flapping() {
        let now = Instant::now();
        let mut changes = Changes::new(PathKind::Relay, now);
        assert_eq!(changes.take(now), None);
        changes.latest = PathKind::Direct;
        assert_eq!(changes.take(now), Some(PathKind::Direct));
        changes.latest = PathKind::Relay;
        assert_eq!(changes.take(now), None);
        changes.latest = PathKind::Direct;
        assert_eq!(changes.take(now + Duration::from_secs(5)), None);
        changes.latest = PathKind::Mixed;
        assert_eq!(
            changes.take(now + Duration::from_secs(5)),
            Some(PathKind::Mixed)
        );
        assert_eq!(changes.take(now + Duration::from_secs(10)), None);
    }
}
