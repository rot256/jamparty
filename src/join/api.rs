//! What the joiner and its pages say to each other on `api/socket`, one
//! JSON text frame each.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    core::{
        ids::{CommentId, EndpointKey},
        settings::SettingsChange,
    },
    join::{
        preferences::PageSettings,
        state::{Answer, Link, Snapshot},
    },
    protocol::{
        comment::{CommentPage, CommentSelector},
        control::{CommentRef, Revision},
        presence::{PagePath, Person, Position},
    },
};

/// A page's number for a request, echoed in the reply (the page counts
/// up from 1 on each socket).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(test, derive(ts_rs::TS), ts(type = "number"))]
pub(crate) struct RequestNumber(u64);

/// What became of a page's comment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum CommentOutcome {
    /// Queued (or delivered already) as `id`.
    Commented { id: CommentId },
    /// It may not have reached the share, or the share's queue is full:
    /// the page keeps it and sends it again, under the same ref, once
    /// connected.
    Unsent { error: String },
    /// Refused: it goes back to the dialog (over the rate limit, too many
    /// comments on their way, an invalid comment); with a rate limit, how
    /// long to wait.
    Refused {
        error: String,
        #[serde(
            rename = "retryAfterSecs",
            with = "crate::core::secs::optional",
            skip_serializing_if = "Option::is_none"
        )]
        #[cfg_attr(test, ts(rename = "retryAfterSecs", type = "number", optional))]
        retry_after: Option<Duration>,
    },
}

/// What became of a page's settings change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum SettingsOutcome {
    /// Taken for this join; the `settings` message before this reply has
    /// them. The page keeps them for its share.
    Saved,
    /// Not taken (an invalid change): the settings stay as they were.
    Refused { error: String },
}

/// What the joiner sends a page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum ToPage {
    /// Always first: the whole state.
    Hello(Snapshot),
    Link(Link),
    People {
        here: Vec<Person>,
    },
    Mine {
        undelivered: Option<Vec<CommentId>>,
    },
    Settings(PageSettings),
    Reload {
        revision: Revision,
    },
    /// Added to the answers (an id seen before is not shown again).
    Answer(Answer),
    /// Someone's pointer on page `url` (sent only to pages on it), or
    /// (none) they left it.
    Cursor {
        key: EndpointKey,
        url: PagePath,
        position: Option<Position>,
    },
    /// The answer to the page's comment `to`, after the messages that
    /// carry its effect.
    CommentReply {
        to: RequestNumber,
        outcome: CommentOutcome,
    },
    /// The answer to the page's settings change `to`, after the
    /// `settings` message that carries it.
    SettingsReply {
        to: RequestNumber,
        outcome: SettingsOutcome,
    },
    /// Every 15 s; a page that hears nothing for longer reconnects.
    Ping,
}

/// A comment as a page hands it over: its ref (sent again unchanged until
/// the joiner answers) and the draft's fields.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Submission {
    pub(crate) n: RequestNumber,
    #[serde(rename = "ref")]
    pub(crate) reference: CommentRef,
    pub(crate) page: CommentPage,
    pub(crate) selector: CommentSelector,
    #[serde(default)]
    pub(crate) selected_text: Option<String>,
    pub(crate) comment: String,
}

/// A change to the settings, from a page.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct Change {
    pub(crate) n: RequestNumber,
    pub(crate) change: SettingsChange,
}

/// What a page sends the joiner. Parsed strictly: anything else closes
/// the socket.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum FromPage {
    /// The page is here (none: it cannot say) and in front: sent first,
    /// then on every move, focus and return to view.
    Page {
        url: Option<PagePath>,
    },
    /// The pointer on the page last named moved, or (none) left it.
    Cursor {
        position: Option<Position>,
    },
    Comment(Submission),
    Settings(Change),
}

/// The TypeScript the page's client is checked against
/// (`client/src/generated/api.ts`), generated from the types above. The
/// test fails when the file is stale; `JAMPOT_UPDATE_BINDINGS=1 cargo test
/// bindings` rewrites it.
#[cfg(test)]
mod bindings {
    use std::{env, fs, path::Path};

    use ts_rs::{Config, TS};

    use crate::{
        core::{
            ids::{CommentId, EndpointKey, Fingerprint},
            name::{JoinerName, MAX_NAME},
            settings::{
                Corner, KeyBinding, Keys, KeysFields, MODIFIER_KEYS, PAGE_KEYS, RESERVED_KEYS,
                SettingsChange, Switch,
            },
        },
        join::{
            api::{
                Change, CommentOutcome, FromPage, RequestNumber, SettingsOutcome, Submission,
                ToPage,
            },
            preferences::PageSettings,
            state::{Answer, Link, MAX_ANSWERS, Snapshot},
        },
        protocol::{
            comment::{Author, CommentPage, CommentSelector, MAX_COMMENT, MAX_SELECTED_TEXT},
            control::{CommentRef, RefusedReason, Revision, StatusLine},
            presence::{
                Colour, CursorSelector, FRACTION_ONE, Fraction, MAX_CURSOR_SELECTOR, MAX_PAGE_PATH,
                PagePath, Person, Position,
            },
        },
        share::hub::MAX_JOINERS,
    };

    fn generate() -> String {
        // Numbers in this API (revisions, counts, seconds) stay far below 2^53.
        let cfg = Config::new().with_large_int("number");
        let decls = [
            CommentId::decl(&cfg),
            CommentRef::decl(&cfg),
            CommentPage::decl(&cfg),
            CommentSelector::decl(&cfg),
            Revision::decl(&cfg),
            EndpointKey::decl(&cfg),
            Fingerprint::decl(&cfg),
            JoinerName::decl(&cfg),
            StatusLine::decl(&cfg),
            RefusedReason::decl(&cfg),
            Switch::decl(&cfg),
            Corner::decl(&cfg),
            KeyBinding::decl(&cfg),
            KeysFields::decl(&cfg),
            Keys::decl(&cfg),
            Author::decl(&cfg),
            PagePath::decl(&cfg),
            Colour::decl(&cfg),
            Person::decl(&cfg),
            CursorSelector::decl(&cfg),
            Fraction::decl(&cfg),
            Position::decl(&cfg),
            PageSettings::decl(&cfg),
            SettingsChange::decl(&cfg),
            Answer::decl(&cfg),
            Link::decl(&cfg),
            Snapshot::decl(&cfg),
            RequestNumber::decl(&cfg),
            CommentOutcome::decl(&cfg),
            SettingsOutcome::decl(&cfg),
            ToPage::decl(&cfg),
            Submission::decl(&cfg),
            Change::decl(&cfg),
            FromPage::decl(&cfg),
        ];
        let mut out = String::from(
            "// Generated from the Rust types by `cargo test` (src/join/api.rs); do not edit.\n\
             // Regenerate: JAMPOT_UPDATE_BINDINGS=1 cargo test bindings\n\n",
        );
        // The limits the share enforces, for the client to stay within.
        let limits = [
            ("MAX_NAME", MAX_NAME),
            ("MAX_COMMENT", MAX_COMMENT),
            ("MAX_SELECTED_TEXT", MAX_SELECTED_TEXT),
            ("MAX_PAGE_PATH", MAX_PAGE_PATH),
            ("MAX_CURSOR_SELECTOR", MAX_CURSOR_SELECTOR),
            ("FRACTION_ONE", usize::from(FRACTION_ONE)),
            // What the joiner keeps, and how many people a share takes.
            ("MAX_ANSWERS", MAX_ANSWERS),
            ("MAX_JOINERS", MAX_JOINERS),
        ];
        for (name, value) in limits {
            out.push_str(&format!("export const {name} = {value};\n"));
        }
        // The keys a binding may not be, as the joiner checks them.
        let keys: [(&str, &[&str]); 3] = [
            ("RESERVED_KEYS", &RESERVED_KEYS),
            ("MODIFIER_KEYS", &MODIFIER_KEYS),
            ("PAGE_KEYS", &PAGE_KEYS),
        ];
        for (name, keys) in keys {
            let keys: Vec<String> = keys
                .iter()
                .map(|key| serde_json::to_string(key).unwrap())
                .collect();
            out.push_str(&format!(
                "export const {name}: readonly string[] = [{}];\n",
                keys.join(", ")
            ));
        }
        for decl in decls {
            out.push_str("\nexport ");
            out.push_str(&decl);
            out.push('\n');
        }
        out
    }

    #[test]
    fn bindings_are_current() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("client/src/generated/api.ts");
        let generated = generate();
        if env::var_os("JAMPOT_UPDATE_BINDINGS").is_some() {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &generated).unwrap();
            return;
        }
        let current = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == generated,
            "{} is stale: run JAMPOT_UPDATE_BINDINGS=1 cargo test bindings",
            path.display()
        );
    }
}
