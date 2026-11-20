//! Serves files, directory indexes and listings within a static share.

use std::{
    io::{self, ErrorKind},
    path::{Component, Path, PathBuf},
};

use http_body_util::BodyExt;
use hyper::{
    Method, Request, Response, StatusCode,
    header::{self, ACCEPT, CACHE_CONTROL, CONTENT_TYPE},
};
use hyper_staticfile::{ResolveResult, Resolver, ResponseBuilder};
use nix::libc;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::fs;

use crate::{
    adapters::http::{AppBody, BodyError, RequestError, empty_response},
    core::text::Untrusted,
    protocol::control::Revision,
    share::{
        inject::{ClientMode, maybe_inject_client, missing_page},
        listing,
    },
};

#[derive(Clone)]
pub(crate) struct Files {
    root: PathBuf,
    single: Option<PathBuf>,
    resolver: Resolver,
}

impl Files {
    /// `root` must be canonical. `single`, if set, is a file directly under it.
    pub(crate) fn new(root: PathBuf, single: Option<PathBuf>) -> Self {
        Self {
            resolver: Resolver::new(root.clone()),
            root,
            single,
        }
    }

    pub(crate) async fn handle<B>(
        &self,
        request: Request<B>,
        revision: Revision,
    ) -> Result<Response<AppBody>, RequestError> {
        let Self {
            root,
            single,
            resolver,
        } = self;
        let request_method = request.method().clone();
        if !matches!(request_method, Method::GET | Method::HEAD) {
            return Err(RequestError::method_not_allowed("GET, HEAD"));
        }
        let uri_path = request.uri().path();
        let Ok(requested) = RequestedPath::try_from(uri_path) else {
            return Ok(empty_response(StatusCode::NOT_FOUND));
        };
        let access = if let Some(file) = single {
            single_access(uri_path, file, root).await?
        } else {
            path_access(&requested, root).await?
        };
        let missing = async || {
            if is_document_request(&request)
                && (single.is_some()
                    || has_visible_ancestor(&root.join(&requested.path), root).await)
            {
                missing_page(&request_method, revision)
            } else {
                empty_response(StatusCode::NOT_FOUND)
            }
        };
        let resolved = match access {
            Access::Missing => return Ok(missing().await),
            Access::Refused(status) => return Ok(empty_response(status)),
            Access::Listing(path) => {
                let response = listing::response(root, &path, &requested.path, &request_method)
                    .await
                    .map_err(|error| {
                        tracing::warn!(error = %Untrusted(&error), "could not list directory");
                        read_error()
                    })?;
                return maybe_inject_client(response, &request_method, ClientMode::Live(revision))
                    .await;
            }
            Access::Redirect(redirect_to) => ResolveResult::IsDirectory { redirect_to },
            Access::File => resolver.resolve_request(&request).await.map_err(|error| {
                tracing::warn!(error = %Untrusted(&error), "could not resolve static file");
                read_error()
            })?,
        };
        let resolved = match resolved {
            ResolveResult::Found(file) => match fs::canonicalize(root.join(&file.path)).await {
                Ok(path)
                    if served(&path, root) && single.as_ref().is_none_or(|file| *file == path) =>
                {
                    ResolveResult::Found(file)
                }
                Ok(path) => {
                    tracing::warn!(path = %Untrusted(path.display()), "refused a symlink outside the root or to a dotfile");
                    return Ok(empty_response(StatusCode::NOT_FOUND));
                }
                Err(error) if is_missing_path_error(&error) => return Ok(missing().await),
                Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                    ResolveResult::PermissionDenied
                }
                Err(error) => {
                    tracing::warn!(error = %Untrusted(&error), "could not verify resolved static file");
                    return Err(read_error());
                }
            },
            ResolveResult::NotFound => return Ok(missing().await),
            other @ (ResolveResult::MethodNotMatched
            | ResolveResult::PermissionDenied
            | ResolveResult::IsDirectory { .. }) => other,
        };
        let content_type = match &resolved {
            ResolveResult::Found(file) => file.content_type.clone(),
            ResolveResult::MethodNotMatched
            | ResolveResult::NotFound
            | ResolveResult::PermissionDenied
            | ResolveResult::IsDirectory { .. } => None,
        };

        let response = ResponseBuilder::new()
            .request(&request)
            .cache_headers(None)
            .build(resolved)
            .map_err(|error| {
                tracing::warn!(error = %Untrusted(&error), "could not build static file response");
                read_error()
            })?;
        let (parts, body) = response.into_parts();
        let mut response = Response::from_parts(
            parts,
            body.map_err(|error| -> BodyError { Box::new(error) })
                .boxed_unsync(),
        );
        if let Some(content_type) = content_type
            && let Ok(value) = header::HeaderValue::from_str(&content_type)
        {
            response.headers_mut().entry(CONTENT_TYPE).or_insert(value);
        }
        response
            .headers_mut()
            .insert(CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
        maybe_inject_client(response, &request_method, ClientMode::Live(revision)).await
    }
}

fn is_document_request<B>(request: &Request<B>) -> bool {
    let headers = request.headers();
    let dest = headers.get("sec-fetch-dest");
    let mode = headers.get("sec-fetch-mode");
    if dest.is_some() || mode.is_some() {
        return dest.is_none_or(|value| value == "document")
            && mode.is_none_or(|value| value == "navigate");
    }

    headers.get_all(ACCEPT).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value.split(',').any(|entry| {
                let mut parts = entry.split(';').map(str::trim);
                parts.next().is_some_and(|media_type| {
                    media_type.eq_ignore_ascii_case("text/html")
                        || media_type.eq_ignore_ascii_case("application/xhtml+xml")
                }) && parts.all(|parameter| {
                    parameter.split_once('=').is_none_or(|(name, value)| {
                        !name.trim().eq_ignore_ascii_case("q")
                            || value
                                .trim()
                                .parse::<f32>()
                                .is_ok_and(|q| q > 0.0 && q <= 1.0)
                    })
                })
            })
        })
    })
}

fn read_error() -> RequestError {
    RequestError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "could not read served directory",
    )
}

/// The request path, decoded and confined to the root.
struct RequestedPath {
    path: PathBuf,
    is_directory: bool,
}

impl TryFrom<&str> for RequestedPath {
    type Error = std::str::Utf8Error;

    fn try_from(request_path: &str) -> Result<Self, Self::Error> {
        let decoded = percent_encoding::percent_decode_str(request_path).decode_utf8()?;
        let mut path = PathBuf::new();
        for component in Path::new(decoded.as_ref()).components() {
            match component {
                Component::Normal(value) => path.push(value),
                Component::ParentDir => {
                    path.pop();
                }
                Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
            }
        }
        Ok(Self {
            path,
            is_directory: request_path.ends_with('/'),
        })
    }
}

enum Access {
    Missing,
    Refused(StatusCode),
    File,
    Redirect(String),
    Listing(PathBuf),
}

async fn single_access(uri_path: &str, file: &Path, root: &Path) -> Result<Access, RequestError> {
    let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
        return Ok(Access::Refused(StatusCode::NOT_FOUND));
    };
    if uri_path == "/" {
        return Ok(Access::Redirect(format!("/{}", encode_name(name))));
    }
    let matches_name = uri_path.strip_prefix('/').is_some_and(|segment| {
        percent_encoding::percent_decode_str(segment)
            .decode_utf8()
            .is_ok_and(|decoded| decoded == name)
    });
    if !matches_name || !served(file, root) {
        return Ok(Access::Refused(StatusCode::NOT_FOUND));
    }
    match fs::symlink_metadata(file).await {
        Ok(metadata) if metadata.is_file() => match fs::canonicalize(file).await {
            Ok(path) if path == file => Ok(Access::File),
            Ok(_) => Ok(Access::Refused(StatusCode::NOT_FOUND)),
            Err(error) => access_error(error),
        },
        Ok(_) => Ok(Access::Refused(StatusCode::NOT_FOUND)),
        Err(error) => access_error(error),
    }
}

async fn path_access(requested: &RequestedPath, root: &Path) -> Result<Access, RequestError> {
    let Some((path, metadata)) =
        (match checked_visible_entry(&root.join(&requested.path), root).await {
            Ok(entry) => entry,
            Err(error) => return access_error(error),
        })
    else {
        return Ok(Access::Refused(StatusCode::NOT_FOUND));
    };
    if !metadata.is_dir() {
        return Ok(if requested.is_directory {
            Access::Refused(StatusCode::NOT_FOUND)
        } else {
            Access::File
        });
    }
    if !requested.is_directory {
        return Ok(Access::Redirect(directory_url(&requested.path)));
    }
    match visible_child(&path, "index.html", root).await {
        Ok(Some((_, metadata))) if metadata.is_file() => return Ok(Access::File),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {}
        Err(error) => return access_error(error),
    }
    // A directory that cannot be read is refused alike for GET and HEAD.
    match fs::read_dir(&path).await {
        Ok(_) => Ok(Access::Listing(path)),
        Err(error) => access_error(error),
    }
}

/// Resolve the visible entry `name` in `directory`, a resolved directory of
/// the share, before anyone opens it.
pub(crate) async fn visible_child(
    directory: &Path,
    name: &str,
    root: &Path,
) -> io::Result<Option<(PathBuf, std::fs::Metadata)>> {
    match resolve(directory, Path::new(name), root).await {
        Err(error) if is_missing_path_error(&error) => Ok(None),
        result => result,
    }
}

async fn checked_visible_entry(
    path: &Path,
    root: &Path,
) -> io::Result<Option<(PathBuf, std::fs::Metadata)>> {
    let Ok(relative) = path.strip_prefix(root) else {
        return Ok(None);
    };
    resolve(root, relative, root).await
}

/// Resolves `relative` beneath `base`, a resolved directory of the share.
async fn resolve(
    base: &Path,
    relative: &Path,
    root: &Path,
) -> io::Result<Option<(PathBuf, std::fs::Metadata)>> {
    if has_hidden_component(relative) {
        return Ok(None);
    }
    // Resolve one component at a time, so a prefix leading out of the share
    // ends the lookup before anything beneath it is touched: nothing in an
    // excluded directory can answer differently by existing or not.
    let mut path = base.to_path_buf();
    for component in relative.components() {
        path.push(component);
        path = fs::canonicalize(&path).await?;
        if !served(&path, root) {
            return Ok(None);
        }
    }
    let metadata = fs::metadata(&path).await?;
    if metadata.is_file() {
        fs::File::open(&path).await?;
    }
    Ok((metadata.is_file() || metadata.is_dir()).then_some((path, metadata)))
}

pub(crate) fn encode_name(name: &str) -> impl std::fmt::Display + '_ {
    const ENCODE: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    utf8_percent_encode(name, ENCODE)
}

pub(super) fn directory_url(path: &Path) -> String {
    use std::fmt::Write as _;

    let mut url = String::from("/");
    for component in path.components() {
        let _ = write!(
            url,
            "{}/",
            encode_name(&component.as_os_str().to_string_lossy())
        );
    }
    url
}

/// Whether a canonical `path` may be served: inside `root`, with no dotfile
/// or dot-directory on the way (so a symlink cannot lead to one either).
fn served(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .is_ok_and(|inside| !has_hidden_component(inside))
}

/// Check the deepest existing entry, including symlinks with missing targets,
/// before exposing live recovery for a missing path.
async fn has_visible_ancestor(path: &Path, root: &Path) -> bool {
    for ancestor in path.ancestors().take_while(|path| served(path, root)) {
        match fs::symlink_metadata(ancestor).await {
            Ok(_) => {
                return fs::canonicalize(ancestor)
                    .await
                    .is_ok_and(|resolved| served(&resolved, root));
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {}
            Err(_) => return false,
        }
    }
    false
}

fn has_hidden_component(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
}

fn access_error(error: io::Error) -> Result<Access, RequestError> {
    if error.kind() == ErrorKind::PermissionDenied {
        return Ok(Access::Refused(StatusCode::FORBIDDEN));
    }
    if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) {
        return Ok(Access::Missing);
    }
    if is_missing_path_error(&error) {
        return Ok(Access::Refused(StatusCode::NOT_FOUND));
    }
    tracing::warn!(error = %Untrusted(&error), "static path access failed");
    Err(read_error())
}

fn is_missing_path_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::InvalidInput
    ) || matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENAMETOOLONG))
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        fs,
        os::unix::{
            ffi::OsStrExt,
            fs::{PermissionsExt, symlink},
            net::UnixListener,
        },
        path::PathBuf,
    };

    use http_body_util::BodyExt as _;
    use hyper::{Method, Request, StatusCode, header};

    use crate::{
        protocol::control::Revision,
        share::files::{Files, RequestedPath, has_hidden_component, served},
    };

    #[tokio::test]
    async fn lists_serveable_entries_and_keeps_index_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let name = "<script> \"#?%é.txt";
        let encoded = "%3Cscript%3E%20%22%23%3F%25%C3%A9.txt";
        fs::write(root.join(name), "odd name").unwrap();
        fs::write(root.join("z-last.txt"), vec![0; 1536]).unwrap();
        fs::create_dir(root.join("sub #")).unwrap();
        fs::write(root.join(".dot"), "hidden").unwrap();
        fs::write(root.join(OsStr::from_bytes(b"nonunicode-\xff")), "").unwrap();
        symlink(".dot", root.join("hidden-link")).unwrap();
        symlink(outside.path(), root.join("escaping-link")).unwrap();
        symlink("z-last.txt", root.join("visible-link")).unwrap();
        let _socket = UnixListener::bind(root.join("special")).unwrap();
        let files = Files::new(root.clone(), None);

        let response = get(&files, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = body(response).await;
        assert!(html.contains("<title>Index of /</title>"));
        assert!(html.contains(&format!(r#"href="./{encoded}""#)));
        assert!(html.contains("&lt;script&gt; &quot;#?%é.txt"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("1.5 KiB"));
        assert!(html.contains("<time datetime="));
        assert!(html.contains("client.js?live=0"));
        assert!(html.find("sub #/").unwrap() < html.find("&lt;script&gt;").unwrap());
        assert!(html.find("visible-link").unwrap() < html.find("z-last.txt").unwrap());
        for omitted in [
            ".dot",
            "hidden-link",
            "escaping-link",
            "special",
            "nonunicode",
            "Parent directory",
        ] {
            assert!(!html.contains(omitted), "{omitted}");
        }
        assert_eq!(
            body(get(&files, &format!("/{encoded}")).await).await,
            "odd name"
        );
        assert_eq!(
            get(&files, "/nonunicode-%FF").await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(&files, "/special").await.status(),
            StatusCode::NOT_FOUND
        );

        let redirect = get(&files, "/sub%20%23?foo=bar").await;
        assert_eq!(redirect.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(redirect.headers()[header::LOCATION], "/sub%20%23/?foo=bar");
        let listing = body(get(&files, "/sub%20%23/").await).await;
        assert!(listing.contains("<h1>Index of /sub #</h1>"));
        assert!(listing.contains("Parent directory"));

        fs::write(root.join("sub #/index.html"), "<body>Static index</body>").unwrap();
        let index = body(get(&files, "/sub%20%23/").await).await;
        assert!(index.contains("Static index"));
        assert!(index.contains("client.js?live=0"));
        assert!(!index.contains("Parent directory"));

        let head = files
            .handle(
                Request::builder()
                    .method(Method::HEAD)
                    .uri("/")
                    .body(())
                    .unwrap(),
                Revision::default(),
            )
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(
            head.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert!(body(head).await.is_empty());
    }

    #[tokio::test]
    async fn unreadable_files_are_omitted_and_do_not_override_listings() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for name in ["index.html", "private.txt"] {
            let path = root.join(name);
            fs::write(&path, "private").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        }
        fs::write(root.join("public.txt"), "public").unwrap();
        let files = Files::new(root, None);

        let response = get(&files, "/").await;
        assert_eq!(response.status(), StatusCode::OK);
        let html = body(response).await;
        assert!(html.contains("public.txt"));
        assert!(!html.contains("index.html"));
        assert!(!html.contains("private.txt"));
        assert_eq!(
            get(&files, "/index.html").await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(&files, "/private.txt").await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn unreadable_directories_are_refused_for_get_and_head() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let locked = root.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("child.txt"), "private").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let files = Files::new(root, None);

        for method in [Method::GET, Method::HEAD] {
            let request = Request::builder()
                .method(&method)
                .uri("/locked/")
                .body(())
                .unwrap();
            let response = files.handle(request, Revision::default()).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method}");
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn single_file_only_serves_its_exact_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("paper #%.pdf");
        fs::write(&file, "the PDF").unwrap();
        fs::write(root.join("sibling.pdf"), "private").unwrap();
        let files = Files::new(root.clone(), Some(file.clone()));

        let redirect = get(&files, "/").await;
        assert_eq!(redirect.status(), StatusCode::MOVED_PERMANENTLY);
        let location = redirect.headers()[header::LOCATION].to_str().unwrap();
        assert_eq!(location, "/paper%20%23%25.pdf");
        let response = get(&files, location).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/pdf");
        assert_eq!(body(response).await, "the PDF");
        for missing in [
            "/sibling.pdf",
            "/sub/",
            "/sub/../paper%20%23%25.pdf",
            "//paper%20%23%25.pdf",
            "/paper%20%23%25.pdf/",
        ] {
            assert_eq!(
                get(&files, missing).await.status(),
                StatusCode::NOT_FOUND,
                "{missing}"
            );
        }

        fs::remove_file(&file).unwrap();
        symlink("sibling.pdf", &file).unwrap();
        assert_eq!(get(&files, location).await.status(), StatusCode::NOT_FOUND);
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        assert_eq!(get(&files, location).await.status(), StatusCode::NOT_FOUND);

        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join(file.file_name().unwrap()), "outside").unwrap();
        let moved = outside.path().join("moved");
        fs::rename(&root, moved).unwrap();
        symlink(outside.path(), &root).unwrap();
        assert_eq!(get(&files, location).await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_pages_only_recover_for_documents_in_scope() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        symlink(dir.path().parent().unwrap(), root.join("outside")).unwrap();

        for (files, visible, excluded) in [
            (
                Files::new(root.clone(), Some(root.join("paper.pdf"))),
                "/paper.pdf",
                ["/paper.log", "/paper.pdf/"],
            ),
            (
                Files::new(root, None),
                "/missing.html",
                ["/.hidden", "/outside"],
            ),
        ] {
            let cases = [
                (visible, Some("document"), Some("navigate"), "*/*", true),
                (visible, None, None, "text/html, */*;q=0.8", true),
                (visible, Some("image"), Some("no-cors"), "text/html", false),
                (
                    visible,
                    Some("iframe"),
                    Some("navigate"),
                    "text/html",
                    false,
                ),
                (visible, None, Some("cors"), "text/html", false),
                (visible, None, None, "*/*", false),
                (visible, None, None, "text/html;q=0", false),
            ];
            for (path, dest, mode, accept, recovery) in cases.into_iter().chain(
                excluded.map(|path| (path, Some("document"), Some("navigate"), "text/html", false)),
            ) {
                let mut request = Request::builder().uri(path).header(header::ACCEPT, accept);
                if let Some(dest) = dest {
                    request = request.header("sec-fetch-dest", dest);
                }
                if let Some(mode) = mode {
                    request = request.header("sec-fetch-mode", mode);
                }
                let response = files
                    .handle(request.body(()).unwrap(), Revision::default())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
                let html = body(response).await;
                if recovery {
                    assert!(html.contains("client.js?live=0"), "{path}");
                } else {
                    assert!(html.is_empty(), "{path}: {html}");
                }
            }
        }
    }

    #[tokio::test]
    async fn missing_children_of_excluded_symlinks_get_plain_404s() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let hidden = root.join(".hidden");
        fs::create_dir(&hidden).unwrap();
        fs::create_dir(root.join("visible")).unwrap();
        for (link, target) in [("outside", outside.path()), ("hidden", hidden.as_path())] {
            fs::write(target.join("existing.html"), "excluded").unwrap();
            // Traversing into it fails, which must not show either.
            let locked = target.join("locked");
            fs::create_dir(&locked).unwrap();
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            symlink(target, root.join(link)).unwrap();
        }
        let files = Files::new(root, None);

        let request = async |path: &str| {
            let request = Request::builder()
                .uri(path)
                .header("sec-fetch-dest", "document")
                .body(())
                .unwrap();
            let response = files.handle(request, Revision::default()).await.unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            (response.headers().clone(), body(response).await)
        };
        let plain = request("/.hidden/existing.html").await;
        assert!(plain.1.is_empty());
        for link in ["outside", "hidden"] {
            for child in [
                "existing.html",
                "missing.html",
                "absent/nested.html",
                "locked/nested.html",
            ] {
                let path = format!("/{link}/{child}");
                assert_eq!(request(&path).await, plain, "{path}");
            }
        }
        assert!(
            request("/visible/missing.html")
                .await
                .1
                .contains("client.js?live=0")
        );
        for target in [outside.path(), hidden.as_path()] {
            fs::set_permissions(target.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    async fn get(files: &Files, uri: &str) -> hyper::Response<crate::adapters::http::AppBody> {
        files
            .handle(
                Request::builder().uri(uri).body(()).unwrap(),
                Revision::default(),
            )
            .await
            .unwrap()
    }

    async fn body(response: hyper::Response<crate::adapters::http::AppBody>) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn only_paths_inside_the_root_without_dotfiles_are_served() {
        let root = PathBuf::from("/srv/.sites/a");
        assert!(served(&root.join("index.html"), &root));
        assert!(served(&root.join("css/site.css"), &root));
        assert!(served(&root, &root));
        assert!(!served(&root.join(".env"), &root));
        assert!(!served(&root.join(".git/config"), &root));
        assert!(!served(&PathBuf::from("/etc/passwd"), &root));
        assert!(!served(&PathBuf::from("/srv/.sites/ab/x"), &root));
    }

    #[test]
    fn sanitizes_paths_without_leaving_the_root() {
        let requested = RequestedPath::try_from("/docs/../../index.html").unwrap();
        assert_eq!(requested.path, PathBuf::from("index.html"));
        assert!(!requested.is_directory);

        let encoded = RequestedPath::try_from("/%2e%2e/%2eenv").unwrap();
        assert_eq!(encoded.path, PathBuf::from(".env"));
        assert!(has_hidden_component(&encoded.path));
        assert!(RequestedPath::try_from("/a/").unwrap().is_directory);
    }
}
