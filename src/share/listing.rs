use std::{
    fmt::Write as _,
    fs::Metadata,
    io::{self, ErrorKind},
    path::Path,
};

use hyper::{
    Method, Response,
    header::{CONTENT_TYPE, HeaderValue},
};
use jiff::Timestamp;
use tokio::fs;

use crate::{
    adapters::http::{AppBody, full_body},
    share::files::{directory_url, encode_name, visible_child},
};

const MAX_ENTRIES: usize = 1_000;
const SCAN_BUDGET: usize = 10_000;

pub(crate) async fn response(
    root: &Path,
    directory: &Path,
    requested: &Path,
    method: &Method,
) -> io::Result<Response<AppBody>> {
    let html = if *method == Method::HEAD {
        String::new()
    } else {
        render(root, directory, requested, MAX_ENTRIES, SCAN_BUDGET).await?
    };
    let mut response = Response::new(full_body(html.into()));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    Ok(response)
}

async fn render(
    root: &Path,
    directory: &Path,
    requested: &Path,
    limit: usize,
    scan_budget: usize,
) -> io::Result<String> {
    let mut entries = Vec::new();
    let mut truncated = false;
    let mut reader = fs::read_dir(directory).await?;
    let mut scanned = 0;
    while scanned < scan_budget {
        let Some(entry) = reader.next_entry().await? else {
            break;
        };
        scanned += 1;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let metadata = match visible_child(directory, &name, root).await {
            Ok(Some((_, metadata))) => metadata,
            Ok(None) => continue,
            Err(error) if error.kind() == ErrorKind::PermissionDenied => continue,
            Err(error) => return Err(error),
        };
        if requested.as_os_str().is_empty() && name == "_jamparty" && metadata.is_dir() {
            continue;
        }
        if entries.len() == limit {
            truncated = true;
            break;
        }
        entries.push((name, metadata));
    }
    entries.sort_unstable_by(|(left, left_meta), (right, right_meta)| {
        right_meta
            .is_dir()
            .cmp(&left_meta.is_dir())
            .then(left.cmp(right))
    });

    let path = escape_html(&requested.to_string_lossy());
    let mut html = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Index of /{path}</title><style>\
         :root {{color-scheme: light dark}}\
         body {{font: 1rem system-ui, sans-serif; max-width: 65rem; margin: 2rem auto; padding: 0 1rem}}\
         h1, td:first-child {{overflow-wrap: anywhere}}\
         table {{width: 100%; border-collapse: collapse; text-align: left}}\
         th, td {{padding: .5rem; border-bottom: 1px solid GrayText}}\
         </style><body><main><h1>Index of /{path}</h1>"
    );
    if let Some(parent) = requested.parent() {
        let _ = write!(
            html,
            "<p><a href=\"{}\">Parent directory</a></p>",
            directory_url(parent)
        );
    }
    if truncated {
        let _ = write!(html, "<p>Listing truncated to {limit} entries.</p>");
    }
    if scanned == scan_budget {
        let _ = write!(
            html,
            "<p>Listing stopped after scanning {scan_budget} entries.</p>"
        );
    }
    html.push_str(
        "<table><thead><tr><th scope=\"col\">Name</th><th scope=\"col\">Size</th>\
         <th scope=\"col\">Modified (UTC)</th></tr></thead><tbody>",
    );
    for (name, metadata) in entries {
        write_entry(&mut html, &name, &metadata);
    }
    html.push_str("</tbody></table></main></body></html>");
    Ok(html)
}

fn write_entry(html: &mut String, name: &str, metadata: &Metadata) {
    let link = encode_name(name);
    let name = escape_html(name);
    let (suffix, size) = if metadata.is_dir() {
        ("/", "—".to_owned())
    } else {
        ("", human_size(metadata.len()))
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| Timestamp::try_from(time).ok());
    let _ = write!(
        html,
        "<tr><td><a href=\"./{link}{suffix}\">{name}{suffix}</a></td><td>{size}</td><td>"
    );
    if let Some(time) = modified {
        let display = time.strftime("%Y-%m-%d %H:%M:%S");
        let _ = write!(html, "<time datetime=\"{time}\">{display}</time>");
    } else {
        html.push('—');
    }
    html.push_str("</td></tr>");
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    for unit in ["KiB", "MiB", "GiB", "TiB", "PiB"] {
        size /= 1024.0;
        if size < 1024.0 {
            return format!("{size:.1} {unit}");
        }
    }
    format!("{:.1} EiB", size / 1024.0)
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::share::listing::render;

    #[tokio::test]
    async fn scan_budget_counts_skipped_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for name in [".one", ".two", ".three"] {
            fs::write(root.join(name), "").unwrap();
        }
        let html = render(&root, &root, "".as_ref(), 10, 2).await.unwrap();
        assert!(!html.contains("<tr><td>"));
        assert!(html.contains("Listing stopped after scanning 2 entries."));
    }

    #[tokio::test]
    async fn limits_the_number_of_visible_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join(".hidden"), "").unwrap();
        for name in ["a", "b", "c"] {
            fs::write(root.join(name), "").unwrap();
        }
        let exact = render(&root, &root, "".as_ref(), 3, 10).await.unwrap();
        assert_eq!(exact.matches("<tr><td>").count(), 3);
        assert!(!exact.contains("truncated"));
        let capped = render(&root, &root, "".as_ref(), 2, 10).await.unwrap();
        assert_eq!(capped.matches("<tr><td>").count(), 2);
        assert!(capped.contains("Listing truncated to 2 entries."));
    }
}
