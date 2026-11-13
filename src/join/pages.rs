//! The pages (browser tabs) attached to the joiner, one per socket: where
//! each is, which was in front last, and the queue of messages to it. The
//! joiner is where its most recently active page is.

use std::collections::HashMap;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use crate::{join::api::ToPage, protocol::presence::PagePath};

/// Pages one joiner takes at once; a socket past this is refused.
const MAX_PAGES: usize = 64;
/// Messages waiting for one page. A page that falls this far behind (a
/// frozen tab) is dropped: it reconnects and starts from a new `hello`.
pub(crate) const PAGE_QUEUE: usize = 256;

/// One attached page, for as long as its socket is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PageId(u64);

/// What a socket's writer reads: the page's messages, and word when the
/// page is dropped for falling behind.
pub(crate) struct Attached {
    pub(crate) id: PageId,
    pub(crate) queue: mpsc::Receiver<Bytes>,
    /// Ends when the joiner drops the page.
    pub(crate) dropped: oneshot::Receiver<()>,
}

/// Too many pages are attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Full;

/// Whether sending dropped a page that fell behind: the joiner may be
/// somewhere else now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "a dropped page may have been the one in front"]
pub(crate) enum Dropped {
    None,
    Some,
}

struct Page {
    /// None before the page said where it is, or while it cannot say.
    url: Option<PagePath>,
    /// When it was last in front, on the pages' clock (none: it has not
    /// said where it is yet).
    active: Option<u64>,
    queue: mpsc::Sender<Bytes>,
    _dropped: oneshot::Sender<()>,
}

#[derive(Default)]
pub(crate) struct Pages {
    pages: HashMap<PageId, Page>,
    next: u64,
    clock: u64,
}

impl Pages {
    /// Attaches a new page, `hello` first in its queue.
    pub(crate) fn attach(&mut self, hello: &ToPage) -> Result<Attached, Full> {
        if self.pages.len() >= MAX_PAGES {
            return Err(Full);
        }
        let id = PageId(self.next);
        self.next = self.next.wrapping_add(1);
        let (sender, queue) = mpsc::channel(PAGE_QUEUE);
        // A new queue has room for its hello.
        if let Ok(hello) = frame(hello) {
            let _ = sender.try_send(hello);
        }
        let (dropped_sender, dropped) = oneshot::channel();
        self.pages.insert(
            id,
            Page {
                url: None,
                active: None,
                queue: sender,
                _dropped: dropped_sender,
            },
        );
        Ok(Attached { id, queue, dropped })
    }

    /// The page's socket closed.
    pub(crate) fn detach(&mut self, id: PageId) {
        self.pages.remove(&id);
    }

    /// The page is on `url` now (none: it cannot say), and in front.
    pub(crate) fn visit(&mut self, id: PageId, url: Option<PagePath>) {
        self.clock = self.clock.wrapping_add(1);
        if let Some(page) = self.pages.get_mut(&id) {
            page.url = url;
            page.active = Some(self.clock);
        }
    }

    /// The page is in front, where it is.
    pub(crate) fn activate(&mut self, id: PageId) {
        let url = self.url(id).cloned();
        self.visit(id, url);
    }

    /// Where the page said it is.
    #[must_use]
    pub(crate) fn url(&self, id: PageId) -> Option<&PagePath> {
        self.pages.get(&id).and_then(|page| page.url.as_ref())
    }

    /// The most recently active page, if any page said where it is.
    fn front(&self) -> Option<(&PageId, &Page)> {
        self.pages
            .iter()
            .filter_map(|(id, page)| page.active.map(|active| (active, id, page)))
            .max_by_key(|(active, _, _)| *active)
            .map(|(_, id, page)| (id, page))
    }

    /// Where the joiner is: the page of its most recently active page.
    #[must_use]
    pub(crate) fn current(&self) -> Option<PagePath> {
        self.front().and_then(|(_, page)| page.url.clone())
    }

    /// Whether `id` is the most recently active page.
    #[must_use]
    pub(crate) fn is_current(&self, id: PageId) -> bool {
        self.front().is_some_and(|(front, _)| *front == id)
    }

    /// Sends `message` to every page it is for: a cursor only to pages on
    /// its page, and none to a page more than half behind (the next
    /// movement replaces it); everything else to all.
    pub(crate) fn publish(&mut self, message: &ToPage) -> Dropped {
        let Ok(frame) = frame(message) else {
            return Dropped::None;
        };
        let cursor_url = match message {
            ToPage::Cursor { url, .. } => Some(url),
            ToPage::Hello(_)
            | ToPage::Link(_)
            | ToPage::People { .. }
            | ToPage::Mine { .. }
            | ToPage::Settings(_)
            | ToPage::Reload { .. }
            | ToPage::Answer(_)
            | ToPage::CommentReply { .. }
            | ToPage::SettingsReply { .. }
            | ToPage::Ping => None,
        };
        let mut behind = Vec::new();
        for (id, page) in &self.pages {
            match cursor_url {
                Some(url) => {
                    if page.url.as_ref() == Some(url) && page.queue.capacity() > PAGE_QUEUE / 2 {
                        let _ = page.queue.try_send(frame.clone());
                    }
                }
                None => {
                    if page.queue.try_send(frame.clone()).is_err() {
                        behind.push(*id);
                    }
                }
            }
        }
        if behind.is_empty() {
            return Dropped::None;
        }
        for id in behind {
            tracing::debug!(?id, "a page fell behind; dropping it");
            self.detach(id);
        }
        Dropped::Some
    }

    /// Sends `message` to one page; drops the page if it is too far
    /// behind.
    pub(crate) fn send(&mut self, id: PageId, message: &ToPage) -> Dropped {
        let Some(page) = self.pages.get(&id) else {
            return Dropped::None;
        };
        let Ok(frame) = frame(message) else {
            return Dropped::None;
        };
        if page.queue.try_send(frame).is_err() {
            tracing::debug!(?id, "a page fell behind; dropping it");
            self.detach(id);
            return Dropped::Some;
        }
        Dropped::None
    }

    /// How many pages are attached.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.pages.len()
    }
}

/// `message` as a text frame's payload. Serialising these types does not
/// fail (no maps with non-string keys, no fallible `Serialize`); were it
/// ever to, the message is not sent, and the error is logged.
pub(crate) fn frame(message: &ToPage) -> Result<Bytes, serde_json::Error> {
    serde_json::to_vec(message).map(Bytes::from).inspect_err(
        |error| tracing::error!(%error, "a page message could not be serialised; not sent"),
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        core::ids::EndpointKey,
        join::{
            api::ToPage,
            pages::{Dropped, MAX_PAGES, PAGE_QUEUE, Pages},
        },
        protocol::presence::PagePath,
    };

    fn url(path: &str) -> Option<PagePath> {
        Some(path.to_owned().try_into().unwrap())
    }

    #[test]
    fn the_most_recently_active_page_wins() {
        let mut pages = Pages::default();
        let a = pages.attach(&ToPage::Ping).unwrap();
        let b = pages.attach(&ToPage::Ping).unwrap();
        // Not placed yet: nowhere.
        assert_eq!(pages.current(), None);
        pages.visit(a.id, url("/a"));
        pages.visit(b.id, url("/b"));
        assert_eq!(pages.current(), url("/b"));
        pages.activate(a.id);
        assert_eq!(pages.current(), url("/a"));
        pages.detach(a.id);
        assert_eq!(pages.current(), url("/b"));
        // A page that cannot say where it is makes the joiner nowhere.
        pages.visit(b.id, None);
        assert_eq!(pages.current(), None);
        pages.detach(b.id);
        assert_eq!(pages.len(), 0);
    }

    #[test]
    fn pages_are_bounded() {
        let mut pages = Pages::default();
        let attached: Vec<_> = (0..MAX_PAGES)
            .map(|_| pages.attach(&ToPage::Ping).unwrap())
            .collect();
        assert!(pages.attach(&ToPage::Ping).is_err());
        pages.detach(attached.first().unwrap().id);
        assert!(pages.attach(&ToPage::Ping).is_ok());
    }

    #[test]
    fn a_page_that_falls_behind_is_dropped() {
        let mut pages = Pages::default();
        let mut slow = pages.attach(&ToPage::Ping).unwrap();
        let mut quick = pages.attach(&ToPage::Ping).unwrap();
        pages.visit(slow.id, url("/"));
        pages.visit(quick.id, url("/"));
        let cursor = ToPage::Cursor {
            key: EndpointKey([1; 32]),
            url: "/".to_owned().try_into().unwrap(),
            position: None,
        };
        for _ in 0..PAGE_QUEUE {
            let _ = pages.publish(&ToPage::Ping);
            // Cursors stop once a page is half behind, never dropping it.
            assert_eq!(pages.publish(&cursor), Dropped::None);
            while quick.queue.try_recv().is_ok() {}
        }
        assert_eq!(pages.len(), 1, "the slow page is dropped");
        assert!(slow.dropped.try_recv().is_err(), "and told so");
        let mut queued = 0;
        while slow.queue.try_recv().is_ok() {
            queued += 1;
        }
        // The hello and the pings that fitted, and cursors only while it
        // was less than half behind.
        assert!(queued <= PAGE_QUEUE, "{queued}");
        assert_eq!(pages.publish(&ToPage::Ping), Dropped::None);
        assert!(quick.queue.try_recv().is_ok());
    }
}
