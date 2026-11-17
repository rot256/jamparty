//! Reminder state shares the queue lock, so listener changes, batch takes and
//! nudge reservations have a single order. A reminder never takes comments.

use std::sync::Arc;

use jiff::Timestamp;
use tokio::time;

use crate::{
    core::{
        queue::{DeliveryState, backoff},
        text::Untrusted,
    },
    share::hub::Hub,
};

#[derive(Default)]
pub(super) struct Nudge {
    /// Reserved or successfully sent for this backlog.
    sent: bool,
    failures: u32,
    pub(super) retry_at: Option<Timestamp>,
}

pub(crate) struct Listening(Arc<Hub>);

impl Drop for Listening {
    fn drop(&mut self) {
        self.0.lock().listeners -= 1;
        self.0.nudge_changed.notify_one();
    }
}

impl Hub {
    /// Present only while a socket request waits for its batch.
    pub(crate) fn listen(self: &Arc<Self>) -> Listening {
        self.lock().listeners += 1;
        self.nudge_changed.notify_one();
        Listening(Arc::clone(self))
    }

    #[cfg(test)]
    pub(crate) fn listeners(&self) -> usize {
        self.lock().listeners
    }

    /// Reserves one reminder. The generation prevents an old command's result
    /// from changing a later backlog while recv continues independently.
    pub(crate) async fn next_nudge(&self) -> (u64, usize) {
        loop {
            let changed = self.nudge_changed.notified();
            let wait = {
                let mut inner = self.lock();
                if inner.listeners == 0
                    && !inner.queue.is_empty()
                    && inner.queue.state() == DeliveryState::Ready
                    && !inner.nudge.sent
                {
                    let now = self.clock.now();
                    match inner.nudge.retry_at.filter(|at| *at > now) {
                        Some(at) => Some(now.duration_until(at).unsigned_abs()),
                        None => {
                            inner.nudge.sent = true;
                            return (inner.generation, inner.queue.len());
                        }
                    }
                } else {
                    None
                }
            };
            match wait {
                Some(wait) => tokio::select! {
                    () = changed => {},
                    () = time::sleep(wait) => {},
                },
                None => changed.await,
            }
        }
    }

    pub(crate) fn finish_nudge(&self, generation: u64, result: Result<(), String>) {
        if let Err(why) = result {
            tracing::warn!(why = %Untrusted(&why), "nudge failed; retrying");
            let mut inner = self.lock();
            if inner.generation == generation {
                inner.nudge.sent = false;
                inner.nudge.failures = inner.nudge.failures.saturating_add(1);
                inner.nudge.retry_at = Some(
                    self.clock
                        .now()
                        .checked_add(backoff(inner.nudge.failures))
                        .unwrap_or(Timestamp::MAX),
                );
            }
        }
        self.nudge_changed.notify_one();
    }
}
