//! komtar's restricted Markdown renderer for answers: raw HTML is shown as
//! text, images are dropped (their alt text stays), and links keep only
//! `http`, `https`, `mailto` and `nabe:` (element links) destinations.

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html};

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "ordinary Markdown events pass through; only unsafe links, images, and raw HTML differ"
)]
pub(crate) fn render(markdown: &str) -> String {
    let options = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(markdown, options);
    let mut filtered = Vec::new();
    let mut suppressed_links = 0_usize;
    let mut suppressed_images = 0_usize;

    for event in parser {
        match event {
            Event::Start(Tag::Link { ref dest_url, .. }) if !safe_link(dest_url) => {
                suppressed_links += 1;
            }
            Event::End(TagEnd::Link) if suppressed_links > 0 => {
                suppressed_links -= 1;
            }
            Event::Start(Tag::Image { .. }) => {
                suppressed_images += 1;
            }
            Event::End(TagEnd::Image) if suppressed_images > 0 => {
                suppressed_images -= 1;
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                filtered.push(Event::Text(CowStr::from(raw.into_string())));
            }
            other => filtered.push(other),
        }
    }

    let mut output = String::new();
    html::push_html(&mut output, filtered.into_iter());
    output
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
        Some("http" | "https" | "mailto" | "nabe")
    )
}

#[cfg(test)]
mod tests {
    use crate::join::markdown::render;

    #[test]
    fn markdown_escapes_html_and_rejects_unsafe_links() {
        let html = render(
            "- one\n- two\n\n<script>alert(1)</script>\n\n[bad](JavaScript:alert(1)) [data](data:text/html,bad) [good](https://example.com) [element](nabe:%23intro%20.item) ![alt text](https://example.com/image.png)",
        );
        assert!(html.contains("<ul>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.to_ascii_lowercase().contains("href=\"javascript:"));
        assert!(!html.contains("href=\"data:"));
        assert!(html.contains("bad"));
        assert!(html.contains("data"));
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains("href=\"nabe:%23intro%20.item\""));
        assert!(html.contains("alt text"));
        assert!(!html.contains("<img"));
    }

    #[test]
    fn relative_links_pass_and_inline_html_is_text() {
        let html = render("[here](/docs?a=b:c) <b onclick=x>bold</b>");
        assert!(html.contains("href=\"/docs?a=b:c\""), "{html}");
        assert!(html.contains("&lt;b onclick=x&gt;"), "{html}");
    }
}
