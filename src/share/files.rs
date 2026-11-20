//! Serves files, directory indexes and listings within a static share.

use std::{
    io::{self, ErrorKind},
    path::{Component, Path, PathBuf},
};

use http_body_util::BodyExt;
use hyper::{
    Method, Request, Response, StatusCode,
    header::{self, CACHE_CONTROL, CONTENT_TYPE},
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
        inject::{ClientMode, maybe_inject_client},
        listing,
    },
};

#[derive(Clone)]
pub(crate) struct Files {
    root: PathBuf,
    resolver: Resolver,
}

impl Files {
    /// `root` must be canonical.
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            resolver: Resolver::new(root.clone()),
            root,
        }
    }

    pub(crate) async fn handle<B>(
        &self,
        request: Request<B>,
        revision: Revision,
    ) -> Result<Response<AppBody>, RequestError> {
        let Self { root, resolver } = self;
        let request_method = request.method().clone();
        if !matches!(request_method, Method::GET | Method::HEAD) {
            return Err(RequestError::method_not_allowed("GET, HEAD"));
        }
        let uri_path = request.uri().path();
        let Ok(requested) = RequestedPath::try_from(uri_path) else {
            return Ok(empty_response(StatusCode::NOT_FOUND));
        };
        let resolved = match path_access(&requested, root).await? {
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
                Ok(path) if served(&path, root) => ResolveResult::Found(file),
                Ok(path) => {
                    tracing::warn!(path = %Untrusted(path.display()), "refused a symlink outside the root or to a dotfile");
                    ResolveResult::NotFound
                }
                Err(error) if is_missing_path_error(&error) => ResolveResult::NotFound,
                Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                    ResolveResult::PermissionDenied
                }
                Err(error) => {
                    tracing::warn!(error = %Untrusted(&error), "could not verify resolved static file");
                    return Err(read_error());
                }
            },
            other @ (ResolveResult::MethodNotMatched
            | ResolveResult::NotFound
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
    Refused(StatusCode),
    File,
    Redirect(String),
    Listing(PathBuf),
}

async fn path_access(requested: &RequestedPath, root: &Path) -> Result<Access, RequestError> {
    let Some((path, metadata)) = (match visible_entry(&root.join(&requested.path), root).await {
        Ok(entry) => entry,
        Err(error) => return access_error(error),
    }) else {
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
    match visible_entry(&path.join("index.html"), root).await {
        Ok(Some((_, metadata))) if metadata.is_file() => Ok(Access::File),
        Ok(_) => Ok(Access::Listing(path)),
        Err(error) if error.kind() == ErrorKind::PermissionDenied => Ok(Access::Listing(path)),
        Err(error) => access_error(error),
    }
}

/// Resolve visible entries before anyone opens them, including index files.
pub(crate) async fn visible_entry(
    path: &Path,
    root: &Path,
) -> io::Result<Option<(PathBuf, std::fs::Metadata)>> {
    if !served(path, root) {
        return Ok(None);
    }
    let result = async {
        let path = fs::canonicalize(path).await?;
        if !served(&path, root) {
            return Ok(None);
        }
        let metadata = fs::metadata(&path).await?;
        if metadata.is_file() {
            fs::File::open(&path).await?;
        }
        Ok((metadata.is_file() || metadata.is_dir()).then_some((path, metadata)))
    }
    .await;
    match result {
        Err(error) if is_missing_path_error(&error) => Ok(None),
        result => result,
    }
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

fn has_hidden_component(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
}

fn access_error(error: io::Error) -> Result<Access, RequestError> {
    if error.kind() == ErrorKind::PermissionDenied {
        return Ok(Access::Refused(StatusCode::FORBIDDEN));
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
        let files = Files::new(root.clone());

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
        let files = Files::new(root);

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
