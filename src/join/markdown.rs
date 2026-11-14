//! komtar's restricted Markdown renderer for answers: raw HTML is shown as
//! text, images are dropped (their alt text stays), and links keep only
//! `http`, `https`, `mailto` and `jampot:` (element links) destinations.
//!
//! The output is bounded: Markdown can grow a lot when rendered (a table
//! header of many columns gives every row as many cells), so an answer
//! whose HTML would pass [`MAX_ANSWER_HTML`] is shown as escaped plain
//! text instead. Rendering stops as soon as the bound is passed. Control
//! characters (other than tab and line breaks) become U+FFFD first: they
//! mean nothing in HTML, and JSON would spell each in six bytes.

use std::{fmt, sync::Arc};

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html};
use serde::{Serialize, Serializer};

/// The most HTML one answer renders to. An answer's text is at most
/// 10,000 characters, so its plain-text fallback (at most six bytes per
/// character, `&quot;`) always fits.
pub(crate) const MAX_ANSWER_HTML: usize = 64 * 1024;

/// An answer's HTML, from [`render`]: at most [`MAX_ANSWER_HTML`] bytes for
/// an answer within the protocol's limit, and cheap to clone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AnswerHtml(Arc<str>);

impl AnswerHtml {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Its size in bytes.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

impl Serialize for AnswerHtml {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// A [`fmt::Write`] that fails once more than [`MAX_ANSWER_HTML`] bytes
/// are written to it, which stops the renderer.
struct Bounded(String);

impl fmt::Write for Bounded {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.0.len().saturating_add(text.len()) > MAX_ANSWER_HTML {
            return Err(fmt::Error);
        }
        self.0.push_str(text);
        Ok(())
    }
}

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "ordinary Markdown events pass through; only unsafe links, images, and raw HTML differ"
)]
#[must_use]
pub(crate) fn render(markdown: &str) -> AnswerHtml {
    let markdown: String = markdown
        .chars()
        .map(|character| match character {
            '\t' | '\n' | '\r' => character,
            control if control.is_control() => char::REPLACEMENT_CHARACTER,
            other => other,
        })
        .collect();
    let markdown = markdown.as_str();
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut suppressed_links = 0_usize;
    let mut suppressed_images = 0_usize;
    // Filtered as they are parsed, never collected: the events of a large
    // table are many more than its text's characters.
    let filtered = Parser::new_ext(markdown, options).filter_map(|event| match event {
        Event::Start(Tag::Link { ref dest_url, .. }) if !safe_link(dest_url) => {
            suppressed_links += 1;
            None
        }
        Event::End(TagEnd::Link) if suppressed_links > 0 => {
            suppressed_links -= 1;
            None
        }
        Event::Start(Tag::Image { .. }) => {
            suppressed_images += 1;
            None
        }
        Event::End(TagEnd::Image) if suppressed_images > 0 => {
            suppressed_images -= 1;
            None
        }
        Event::Html(raw) | Event::InlineHtml(raw) => {
            Some(Event::Text(CowStr::from(raw.into_string())))
        }
        other => Some(other),
    });
    let mut output = Bounded(String::new());
    let html = match html::write_html_fmt(&mut output, filtered) {
        Ok(()) => output.0,
        Err(fmt::Error) => plain(markdown),
    };
    AnswerHtml(Arc::from(html))
}

/// `text` as one escaped paragraph, its line breaks kept.
fn plain(text: &str) -> String {
    let mut html = String::from("<p>");
    for character in text.chars() {
        match character {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            '\n' => html.push_str("<br>\n"),
            other => html.push(other),
        }
    }
    html.push_str("</p>\n");
    html
}

fn safe_link(destination: &str) -> bool {
    let destination = destination.trim();
    let Some(colon) = destination.find(':') else {
        return true;
    };
    let first_separator = destination
        .find(['/', '?', '#'])
        .unwrap_or(destination.len());
    if colon > first_separator {
        return true;
    }
    matches!(
        destination
            .get(..colon)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("http" | "https" | "mailto" | "jampot")
    )
}

#[cfg(test)]
mod tests {
    use crate::join::markdown::{MAX_ANSWER_HTML, plain, render};

    /// A table header of `columns` columns over `rows` one-cell rows: every
    /// row renders as many cells as the header has.
    fn wide_table(columns: usize, rows: usize) -> String {
        let mut markdown = "|a".repeat(columns);
        markdown.push('\n');
        markdown.push_str(&"|-".repeat(columns));
        markdown.push('\n');
        markdown.push_str(&"x\n".repeat(rows));
        markdown
    }

    #[test]
    fn a_table_that_renders_too_large_is_plain_text() {
        // 10,000 characters that render to about 2.4 MB as a table.
        let markdown = wide_table(1250, 2499);
        assert!(markdown.chars().count() <= 10_000);
        let html = render(&markdown);
        assert!(html.len() <= MAX_ANSWER_HTML, "{}", html.len());
        assert!(!html.as_str().contains("<table>"));
        assert!(html.as_str().starts_with("<p>|a|a"), "{}", html.as_str());
        // A table of ordinary size stays a table.
        let html = render(&wide_table(3, 4));
        assert!(html.as_str().contains("<table>"), "{}", html.as_str());
    }

    #[test]
    fn the_plain_fallback_of_the_largest_answer_fits() {
        // The worst case for escaping: every character grows six-fold.
        for character in ['"', '\'', '&', '\n', '\u{10ffff}'] {
            let text = character.to_string().repeat(10_000);
            assert!(plain(&text).len() <= MAX_ANSWER_HTML, "{character:?}");
        }
    }

    #[test]
    fn control_characters_are_replaced() {
        let html = render("a\u{1}b\u{1b}[2Jc\u{85}d\te");
        assert_eq!(html.as_str(), "<p>a\u{fffd}b\u{fffd}[2Jc\u{fffd}d\te</p>\n");
    }

    #[test]
    fn markdown_escapes_html_and_rejects_unsafe_links() {
        let html = render(
            "- one\n- two\n\n<script>alert(1)</script>\n\n[bad](JavaScript:alert(1)) [data](data:text/html,bad) [good](https://example.com) [element](jampot:%23intro%20.item) ![alt text](https://example.com/image.png)",
        );
        let html = html.as_str();
        assert!(html.contains("<ul>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.to_ascii_lowercase().contains("href=\"javascript:"));
        assert!(!html.contains("href=\"data:"));
        assert!(html.contains("bad"));
        assert!(html.contains("data"));
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains("href=\"jampot:%23intro%20.item\""));
        assert!(html.contains("alt text"));
        assert!(!html.contains("<img"));
    }

    #[test]
    fn relative_links_pass_and_inline_html_is_text() {
        let html = render("[here](/docs?a=b:c) <b onclick=x>bold</b>");
        let html = html.as_str();
        assert!(html.contains("href=\"/docs?a=b:c\""), "{html}");
        assert!(html.contains("&lt;b onclick=x&gt;"), "{html}");
    }
}
