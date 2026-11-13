//! Files and attachments, their upload and ranged download.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use penfii_contracts::{CreateFileRequest, FileObject};
use penfii_core::CoreError;
use penfii_core::files::{self, StoredFile};
use uuid::Uuid;

use crate::AppState;
use crate::error::ApiError;
use crate::extract::{Actor, Body as JsonBody};

use axum::Router;
use axum::routing::{delete, get, post, put};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/workspaces/{id}/files", post(create_file))
        .route("/v1/files/{id}/content", put(upload_file).get(file_content))
        .route("/v1/files/{id}", delete(delete_file))
        .route("/v1/attachments/{id}/content", get(attachment_content))
}

async fn create_file(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Actor(user_id): Actor,
    JsonBody(body): JsonBody<CreateFileRequest>,
) -> Result<Response, ApiError> {
    let file = files::begin_upload(
        &state.pool,
        &state.limits,
        user_id,
        workspace_id,
        &body.filename,
        body.byte_size,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(file_object(file))).into_response())
}

async fn upload_file(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
    request: axum::extract::Request<Body>,
) -> Result<Response, ApiError> {
    let Some(store) = state.objects.as_ref() else {
        return Err(CoreError::conflict(
            "storage_unconfigured",
            "object storage is not configured",
        )
        .into());
    };
    let max = usize::try_from(state.limits.file_max_bytes.unwrap_or(0)).unwrap_or(0);
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| CoreError::validation("content length is required"))?;
    if max == 0 || declared == 0 || declared > max {
        return Err(CoreError::validation("file size is outside the configured limit").into());
    }
    // The body is kept in the chunks it arrives in: joining them into one buffer would copy the whole file, and
    // the store writes the chunks as they are. More than was declared is refused as soon as it shows.
    let mut chunks = Vec::new();
    let mut received = 0usize;
    let mut stream = request.into_body().into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| CoreError::validation("file body could not be read"))?;
        received += chunk.len();
        if received > declared {
            return Err(CoreError::validation("file size does not match").into());
        }
        chunks.push(chunk);
    }
    if received != declared {
        return Err(CoreError::validation("file size does not match").into());
    }
    let body = files::UploadBody::from_chunks(chunks);
    let file = files::finish_upload(&state.pool, store, &state.limits, user_id, id, &body).await?;
    Ok(Json(file_object(file)).into_response())
}

async fn delete_file(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    files::delete_upload(&state.pool, state.objects.as_ref(), user_id, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn file_content(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let Some(store) = state.objects.as_ref() else {
        return Err(CoreError::NotFound.into());
    };
    let source = files::locate_owned(&state.pool, user_id, id).await?;
    object_response(store, source, &headers).await
}

async fn attachment_content(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Actor(user_id): Actor,
) -> Result<Response, ApiError> {
    let Some(store) = state.objects.as_ref() else {
        return Err(CoreError::NotFound.into());
    };
    let source = files::locate_attachment(&state.pool, user_id, id).await?;
    object_response(store, source, &headers).await
}

fn file_object(file: StoredFile) -> FileObject {
    FileObject {
        id: file.id,
        generation: file.generation,
        filename: file.filename,
        byte_size: file.byte_size,
        mime: file.detected_mime,
        state: file.state,
    }
}

/// The types a browser may show in place. Everything else is a download, whatever it claims to be:
/// only a type the upload recognised from the file's own bytes can be on this list, and HTML, SVG
/// and the like are not.
const INLINE_MIME: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "video/mp4",
    "video/webm",
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "audio/mp4",
];

/// What a `Range` header asks for, once checked against the object's size.
#[derive(Debug, PartialEq, Eq)]
enum RangeRequest {
    /// No usable range: send everything.
    Whole,
    /// A satisfiable `start..end` (end exclusive).
    Part(std::ops::Range<u64>),
    /// A range beyond the end of the object.
    Unsatisfiable,
}

fn parse_range(header: Option<&str>, size: u64) -> RangeRequest {
    let Some(spec) = header.and_then(|value| value.trim().strip_prefix("bytes=")) else {
        return RangeRequest::Whole;
    };
    // Several ranges are legal but nobody needs them for media; answer with the whole file.
    if spec.contains(',') {
        return RangeRequest::Whole;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return RangeRequest::Whole;
    };
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        // `-n`: the last n bytes.
        let Ok(n) = last.parse::<u64>() else {
            return RangeRequest::Whole;
        };
        if n == 0 || size == 0 {
            return RangeRequest::Unsatisfiable;
        }
        return RangeRequest::Part(size.saturating_sub(n)..size);
    }
    let Ok(start) = first.parse::<u64>() else {
        return RangeRequest::Whole;
    };
    let end = if last.is_empty() {
        size
    } else {
        match last.parse::<u64>() {
            Ok(end) => end.saturating_add(1).min(size),
            Err(_) => return RangeRequest::Whole,
        }
    };
    if start >= size || end <= start {
        return RangeRequest::Unsatisfiable;
    }
    RangeRequest::Part(start..end)
}

async fn object_response(
    store: &penfii_core::files::ObjectStore,
    source: files::FileSource,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let size = store
        .size(&source.storage_key)
        .await
        .map_err(CoreError::Internal)?;
    let range_header = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let (status, range) = match parse_range(range_header, size) {
        RangeRequest::Whole => (StatusCode::OK, 0..size),
        RangeRequest::Part(range) => (StatusCode::PARTIAL_CONTENT, range),
        RangeRequest::Unsatisfiable => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return Ok(response);
        }
    };
    let opened = store
        .open(&source.storage_key, Some(range.clone()))
        .await
        .map_err(CoreError::Internal)?;
    let mut response = bytes_headers(
        &source.filename,
        source.mime.as_deref(),
        Body::from_stream(opened.stream),
    );
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from(range.end - range.start),
    );
    if status == StatusCode::PARTIAL_CONTENT
        && let Ok(value) =
            HeaderValue::from_str(&format!("bytes {}-{}/{size}", range.start, range.end - 1))
    {
        headers.insert(header::CONTENT_RANGE, value);
    }
    Ok(response)
}

#[cfg(test)]
fn bytes_response(filename: &str, mime: Option<&str>, bytes: Vec<u8>) -> Response {
    bytes_headers(filename, mime, Body::from(bytes))
}

fn bytes_headers(filename: &str, mime: Option<&str>, body: Body) -> Response {
    let mime = mime.unwrap_or("application/octet-stream");
    let kind = if INLINE_MIME.contains(&mime) {
        "inline"
    } else {
        "attachment"
    };
    let disposition = content_disposition(kind, filename);
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime)
                    .unwrap_or(HeaderValue::from_static("application/octet-stream")),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition)
                    .unwrap_or(HeaderValue::from_static("attachment")),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            ),
            // A file that somebody uploaded is opened on this origin: it may not run or load anything
            // (the policy GitHub uses for user content).
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
            ),
        ],
        body,
    )
        .into_response()
}

/// `Content-Disposition` for a download. `filename` is the ASCII fallback for old clients and
/// `filename*` (RFC 6266 / 5987) carries the real UTF-8 name, so a Korean file name is saved as
/// written instead of as the spaces that remain after dropping its letters.
fn content_disposition(kind: &str, filename: &str) -> String {
    let fallback = safe_filename(filename);
    let mut disposition = format!("{kind}; filename=\"{fallback}\"");
    let real: String = filename.chars().filter(|ch| !ch.is_control()).collect();
    if !real.is_empty() && !real.is_ascii() {
        disposition.push_str("; filename*=UTF-8''");
        for byte in real.bytes() {
            if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
                disposition.push(byte as char);
            } else {
                disposition.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    disposition
}

/// ASCII only: a letter that cannot be sent as is becomes `_`, so the extension stays readable.
fn safe_filename(filename: &str) -> String {
    let name: String = filename
        .chars()
        .filter(|ch| *ch != '"' && *ch != '\\' && !ch.is_control())
        .map(|ch| if ch.is_ascii() { ch } else { '_' })
        .collect();
    if name.trim_matches(|c| c == '_' || c == ' ').is_empty() {
        "download".into()
    } else {
        name
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    #[test]
    fn an_uploaded_file_is_sandboxed_and_only_images_are_shown_inline() {
        let image = bytes_response("a.png", Some("image/png"), vec![1]);
        let html = bytes_response("a.html", Some("text/html"), vec![1]);
        for response in [&image, &html] {
            let policy = response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap();
            assert!(policy.contains("default-src 'none'") && policy.contains("sandbox"));
        }
        assert!(
            image.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .starts_with("inline")
        );
        assert!(
            html.headers()[header::CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .starts_with("attachment")
        );
    }
}

#[cfg(test)]
mod disposition_tests {
    use super::content_disposition;

    #[test]
    fn a_korean_file_name_is_sent_in_filename_star_with_a_readable_fallback() {
        let value = content_disposition("attachment", "한글 파일 이름.txt");
        assert!(
            value.starts_with("attachment; filename=\"__ __ __.txt\""),
            "{value}"
        );
        assert!(
            value.ends_with(
                "; filename*=UTF-8''%ED%95%9C%EA%B8%80%20%ED%8C%8C%EC%9D%BC%20%EC%9D%B4%EB%A6%84.txt"
            ),
            "{value}"
        );
    }

    #[test]
    fn an_ascii_name_stays_plain_and_a_quote_cannot_end_the_value_early() {
        assert_eq!(
            content_disposition("inline", "diagram.png"),
            "inline; filename=\"diagram.png\""
        );
        let hostile = content_disposition("attachment", "a\".txt\r\nX: y");
        assert!(
            !hostile.contains('\r') && !hostile.contains('\n'),
            "{hostile}"
        );
        assert_eq!(hostile.matches('"').count(), 2, "{hostile}");
    }

    #[test]
    fn a_name_without_any_usable_letter_falls_back_to_download() {
        assert!(
            content_disposition("attachment", "한글")
                .starts_with("attachment; filename=\"download\"")
        );
    }
}

#[cfg(test)]
mod range_tests {
    use super::{INLINE_MIME, RangeRequest, bytes_response, parse_range};
    use axum::http::header;

    #[test]
    fn a_range_is_clamped_or_refused_never_guessed() {
        let part = |a, b| RangeRequest::Part(a..b);
        assert_eq!(parse_range(None, 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("bytes=0-9"), 100), part(0, 10));
        assert_eq!(parse_range(Some("bytes=90-"), 100), part(90, 100));
        assert_eq!(parse_range(Some("bytes=-5"), 100), part(95, 100));
        assert_eq!(parse_range(Some("bytes=-500"), 100), part(0, 100));
        assert_eq!(parse_range(Some("bytes=50-5000"), 100), part(50, 100));
        assert_eq!(
            parse_range(Some("bytes=100-"), 100),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=9-3"), 100),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=-0"), 100),
            RangeRequest::Unsatisfiable
        );
        // Not understood or several at once: the whole file.
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("items=0-1"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("bytes=a-b"), 100), RangeRequest::Whole);
    }

    #[test]
    fn only_listed_types_are_shown_inline() {
        for mime in INLINE_MIME {
            let response = bytes_response("f", Some(mime), vec![1]);
            assert!(
                response.headers()[header::CONTENT_DISPOSITION]
                    .to_str()
                    .unwrap()
                    .starts_with("inline"),
                "{mime}"
            );
        }
        for mime in [
            "text/html",
            "image/svg+xml",
            "text/csv",
            "application/pdf",
            "video/quicktime",
            "application/octet-stream",
            "video/x-matroska",
        ] {
            let response = bytes_response("f", Some(mime), vec![1]);
            assert!(
                response.headers()[header::CONTENT_DISPOSITION]
                    .to_str()
                    .unwrap()
                    .starts_with("attachment"),
                "{mime}"
            );
        }
    }
}
