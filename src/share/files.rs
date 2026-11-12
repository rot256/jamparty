//! Serves the shared directory: files and directory `index.html`s, never
//! dotfiles or anything outside the root.

use std::path::{Component, Path, PathBuf};

use http_body_util::BodyExt;
use hyper::{
    Method, Request, Response, StatusCode,
    body::Incoming,
    header::{self, CACHE_CONTROL, CONTENT_TYPE},
};
use hyper_staticfile::{ResolveResult, Resolver, ResponseBuilder};

use crate::{
    BoxError,
    share::{
        body::{AppBody, RequestError, empty_response},
        inject::{ClientMode, maybe_inject_client},
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

    pub(crate) async fn handle(
        &self,
        request: Request<Incoming>,
        revision: u64,
    ) -> Result<Response<AppBody>, RequestError> {
        let Self { root, resolver } = self;
        let request_method = request.method().clone();
        if !matches!(request_method, Method::GET | Method::HEAD) {
            return Err(RequestError::method_not_allowed("GET, HEAD"));
        }
        if let Some(status) = path_access(request.uri().path(), root).await? {
            return Ok(empty_response(status));
        }
        let resolved = resolver.resolve_request(&request).await.map_err(|error| {
            tracing::warn!(%error, "could not resolve static file");
            read_error()
        })?;
        let resolved = match resolved {
            ResolveResult::Found(file) => {
                match tokio::fs::canonicalize(root.join(&file.path)).await {
                    Ok(path) if served(&path, root) => ResolveResult::Found(file),
                    Ok(path) => {
                        tracing::warn!(path = %path.display(), "refused a symlink outside the root or to a dotfile");
                        ResolveResult::NotFound
                    }
                    Err(error) if is_missing_path_error(&error) => ResolveResult::NotFound,
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                        ResolveResult::PermissionDenied
                    }
                    Err(error) => {
                        tracing::warn!(%error, "could not verify resolved static file");
                        return Err(read_error());
                    }
                }
            }
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
                tracing::warn!(%error, "could not build static file response");
                read_error()
            })?;
        let (parts, body) = response.into_parts();
        let mut response = Response::from_parts(
            parts,
            body.map_err(|error| -> BoxError { Box::new(error) })
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

impl From<&str> for RequestedPath {
    fn from(request_path: &str) -> Self {
        let decoded = percent_encoding::percent_decode_str(request_path).decode_utf8_lossy();
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
        Self {
            path,
            is_directory: request_path.ends_with('/'),
        }
    }
}

/// `None` if the path may be served, else the status to answer with.
async fn path_access(request_path: &str, root: &Path) -> Result<Option<StatusCode>, RequestError> {
    let requested = RequestedPath::from(request_path);
    if has_hidden_component(&requested.path) {
        return Ok(Some(StatusCode::NOT_FOUND));
    }
    let path = match tokio::fs::canonicalize(root.join(&requested.path)).await {
        Ok(path) => path,
        Err(error) => return access_error(error),
    };
    if !served(&path, root) {
        return Ok(Some(StatusCode::NOT_FOUND));
    }
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) => return access_error(error),
    };
    if metadata.is_file() && !requested.is_directory {
        return Ok(None);
    }
    if !metadata.is_dir() {
        return Ok(Some(StatusCode::NOT_FOUND));
    }
    if !requested.is_directory {
        return Ok(None);
    }
    let index = match tokio::fs::canonicalize(path.join("index.html")).await {
        Ok(index) => index,
        Err(error) => return access_error(error),
    };
    if !served(&index, root) {
        return Ok(Some(StatusCode::NOT_FOUND));
    }
    match tokio::fs::metadata(index).await {
        Ok(metadata) if metadata.is_file() => Ok(None),
        Ok(_) => Ok(Some(StatusCode::NOT_FOUND)),
        Err(error) => access_error(error),
    }
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

fn access_error(error: std::io::Error) -> Result<Option<StatusCode>, RequestError> {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return Ok(Some(StatusCode::FORBIDDEN));
    }
    if is_missing_path_error(&error) {
        return Ok(Some(StatusCode::NOT_FOUND));
    }
    tracing::warn!(%error, "static path access failed");
    Err(read_error())
}

fn is_missing_path_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::NotADirectory
            | std::io::ErrorKind::InvalidInput
    ) || matches!(
        error.raw_os_error(),
        Some(nix::libc::ELOOP | nix::libc::ENAMETOOLONG)
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::share::files::{RequestedPath, has_hidden_component, served};

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
        let requested = RequestedPath::from("/docs/../../index.html");
        assert_eq!(requested.path, PathBuf::from("index.html"));
        assert!(!requested.is_directory);

        let encoded = RequestedPath::from("/%2e%2e/%2eenv");
        assert_eq!(encoded.path, PathBuf::from(".env"));
        assert!(has_hidden_component(&encoded.path));
        assert!(RequestedPath::from("/a/").is_directory);
    }
}
