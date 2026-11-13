//! Tells the share where this joiner is and where its pointer is: one
//! task, so presence and cursors go out in order. Presence goes out as it
//! changes, at most once per [`REPORT_GAP`] (a burst ends on its last
//! state); the latest cursor follows once the share has its page.

use std::{
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use tokio::{
    sync::Notify,
    time::{self, Instant},
};

use crate::{
    join::{control::Control, session::Session},
    protocol::presence::{PagePath, Position, Whereabouts},
};

/// The least time between two presence reports: at most 4 per second.
/// Everything goes through [`run`], so nothing sends presence faster; the
/// share disconnects a joiner past 10 per second sustained
/// (`RateLimit::PRESENCE_FLOOD`), well above.
pub(crate) const REPORT_GAP: Duration = Duration::from_millis(250);

/// The pointer to send: on which page, and where (none: it left).
type Pointer = (PagePath, Option<Position>);

/// What [`run`] is asked to send.
#[derive(Default)]
pub(crate) struct Uplink {
    wake: Notify,
    /// The latest pointer not sent yet; a newer one replaces it.
    pointer: Mutex<Option<Pointer>>,
}

impl Uplink {
    fn pointer(&self) -> MutexGuard<'_, Option<Pointer>> {
        self.pointer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The whereabouts may have changed (a page, the name, the switch, the
    /// connection).
    pub(crate) fn changed(&self) {
        self.wake.notify_one();
    }

    /// The pointer on page `url` moved, or left it.
    pub(crate) fn cursor(&self, url: PagePath, position: Option<Position>) {
        *self.pointer() = Some((url, position));
        self.wake.notify_one();
    }

    fn take_pointer(&self) -> Option<Pointer> {
        self.pointer().take()
    }
}

/// What the share was told, and on which connection.
struct Told {
    control: Arc<Control>,
    whereabouts: Whereabouts,
}

impl Told {
    fn is(&self, control: &Arc<Control>, whereabouts: &Whereabouts) -> bool {
        Arc::ptr_eq(&self.control, control) && self.whereabouts == *whereabouts
    }
}

/// Sends the session's whereabouts and pointer to the share until the
/// task is dropped.
pub(crate) async fn run(session: Arc<Session>) {
    let mut told: Option<Told> = None;
    let mut last: Option<Instant> = None;
    loop {
        session.uplink().wake.notified().await;
        loop {
            let Some(control) = session.control() else {
                break;
            };
            let now = session.whereabouts();
            if !told.as_ref().is_some_and(|told| told.is(&control, &now)) {
                if let Some(next) = last.map(|last| last + REPORT_GAP)
                    && Instant::now() < next
                {
                    time::sleep_until(next).await;
                    // What to tell may have changed meanwhile.
                    continue;
                }
                last = Some(Instant::now());
                if let Err(error) = control.tell(now.clone()).await {
                    tracing::debug!(%error, "could not report presence");
                    break;
                }
                told = Some(Told {
                    control: Arc::clone(&control),
                    whereabouts: now.clone(),
                });
            }
            if let Some((url, position)) = session.uplink().take_pointer()
                // Only for the page the share has; one left behind is moot.
                && now.url.as_ref() == Some(&url)
                && let Err(error) = control.cursor(position).await
            {
                tracing::debug!(%error, "could not send the cursor");
            }
            if told
                .as_ref()
                .is_some_and(|told| told.is(&control, &session.whereabouts()))
            {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        join::uplink::Uplink,
        protocol::presence::{CursorSelector, Fraction, Position},
    };

    fn position(x: u16) -> Option<Position> {
        Some(Position {
            selector: CursorSelector::try_from("h1".to_owned()).unwrap(),
            x: Fraction::try_from(x).unwrap(),
            y: Fraction::try_from(0).unwrap(),
        })
    }

    #[test]
    fn only_the_latest_pointer_waits() {
        let uplink = Uplink::default();
        let page = "/a".to_owned().try_into().unwrap();
        uplink.cursor("/old".to_owned().try_into().unwrap(), position(1));
        uplink.cursor(page, position(2));
        let (url, latest) = uplink.take_pointer().unwrap();
        assert_eq!(String::from(url), "/a");
        assert_eq!(latest, position(2));
        assert!(uplink.take_pointer().is_none());
    }
}
