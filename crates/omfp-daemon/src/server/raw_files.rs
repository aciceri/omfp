//! Raw workspace files (DMN-250..DMN-254): `GET <fileBase>/<workspaceId>/<path>` serves the bytes of a workspace file
//! for `<video>`, `<audio>`, `<img>` and `fetch` (`useFileUrl`, UI-540), which cannot carry the token in a header.
//!
//! `<fileBase>` is `/files/<key>`, the key a random 128-bit secret made at startup and given only to authenticated
//! clients in `system.hello` (PRT-150), like the widget key (DMN-084) but distinct from it: a URL that leaks (a page
//! source, a log) opens the workspace files until the daemon restarts, never the protocol. The path is resolved as
//! `workspaces.read_file` resolves it (DMN-089), an absolute path outside the folder included (DMN-314); every
//! failure — a wrong key, an unknown workspace, a relative path leaving the folder, a missing file, a folder — is the
//! same `404`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{self, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha1::{Digest, Sha1};
use subtle::ConstantTimeEq;

use super::AppState;
use crate::workspaces::image_mime;

/// What `fileBase` starts with; the key follows.
pub(crate) const PREFIX: &str = "/files/";

/// Bytes read to tell a file's type when its extension does not.
const SNIFF: usize = 512;

/// Bytes read and sent at a time.
const CHUNK: u64 = 64 * 1024;

/// What a request needs of the file it names.
struct Opened {
    file: File,
    size: u64,
    modified: Option<SystemTime>,
    content_type: &'static str,
}

/// `GET <fileBase>/<workspaceId>/<path>` (DMN-250): the query string is ignored, so a client may add one to load a
/// changed file again, except `rev=HEAD`, which asks for the file's bytes at `HEAD` (DMN-255).
pub(super) async fn serve(
    State(state): State<Arc<AppState>>,
    extract::Path(rest): extract::Path<String>,
    extract::RawQuery(query): extract::RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some((key, rest)) = rest.split_once('/') else {
        return not_found();
    };
    let expected = state.file_base.strip_prefix(PREFIX).unwrap_or_default();
    if !bool::from(expected.as_bytes().ct_eq(key.as_bytes())) {
        return not_found();
    }
    let Some((workspace, path)) = rest.split_once('/') else {
        return not_found();
    };
    let workspaces = Arc::clone(&state.services.workspaces);
    let (workspace, path) = (workspace.to_owned(), path.to_owned());
    if query.is_some_and(|query| query.split('&').any(|pair| pair == "rev=HEAD")) {
        let read =
            tokio::task::spawn_blocking(move || workspaces.read_head(&workspace, &path)).await;
        return match read {
            Ok(Ok(Some((bytes, relative)))) => respond_head(bytes, &relative, &headers),
            _ => not_found(),
        };
    }
    let opened = tokio::task::spawn_blocking(move || {
        let (mut file, relative) = workspaces.open_file(&workspace, &path).ok()?;
        let metadata = file.metadata().ok()?;
        let mut start = Vec::with_capacity(SNIFF);
        (&mut file)
            .take(SNIFF as u64)
            .read_to_end(&mut start)
            .ok()?;
        Some(Opened {
            file,
            size: metadata.len(),
            modified: metadata.modified().ok(),
            content_type: content_type(&relative, &start),
        })
    })
    .await;
    match opened {
        Ok(Some(opened)) => respond(opened, &headers),
        _ => not_found(),
    }
}

/// The bytes of a file at `HEAD` (DMN-255), whole: typed like the file (DMN-251), with the hex SHA-1 of the bytes as
/// a strong validator (`304` when the client's copy has it) and the headers of every file (DMN-254); no ranges.
fn respond_head(bytes: Vec<u8>, relative: &Path, headers: &HeaderMap) -> Response {
    let tag = format!("\"{}\"", hex::encode(Sha1::digest(&bytes)));
    let current = header_text(headers, header::IF_NONE_MATCH)
        .is_some_and(|value| value.split(',').any(|named| named.trim() == tag));
    let content_type = content_type(relative, &bytes[..bytes.len().min(SNIFF)]);
    let mut response = if current {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
    };
    shared_headers(response.headers_mut(), &tag, "none");
    response
}

/// The headers of every file response (DMN-254), with its strong validator `tag` and whether it serves `ranges`.
fn shared_headers(out: &mut HeaderMap, tag: &str, ranges: &'static str) {
    insert(out, header::ETAG, tag);
    for (name, value) in [
        (header::ACCEPT_RANGES, ranges),
        // Revalidated on every use, with the validators the response carries.
        (header::CACHE_CONTROL, "private, no-cache"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        // A workspace's HTML or SVG opened at its URL runs nothing in the daemon's origin (DMN-254).
        (header::CONTENT_SECURITY_POLICY, "sandbox"),
        (
            header::HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin",
        ),
    ] {
        out.insert(name, HeaderValue::from_static(value));
    }
}

fn not_found() -> Response {
    ([(header::CACHE_CONTROL, "no-store")], StatusCode::NOT_FOUND).into_response()
}

/// The response to a request with `headers` for an opened file: `304` when the client's copy is current (DMN-253),
/// `206` for one satisfiable range, `416` for an unsatisfiable one, else `200` with every byte (DMN-252).
fn respond(opened: Opened, headers: &HeaderMap) -> Response {
    let Opened {
        file,
        size,
        modified,
        content_type,
    } = opened;
    let tag = etag(size, modified);
    let last_modified = modified.map(httpdate::fmt_http_date);
    let mut response = if not_modified(headers, &tag, modified) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let range = if if_range_holds(headers, &tag, last_modified.as_deref()) {
            header_text(headers, header::RANGE).map_or(Range::Whole, |value| range(value, size))
        } else {
            Range::Whole
        };
        match range {
            Range::Whole => body(file, 0, size, content_type).into_response(),
            Range::Part { start, end } => {
                let mut part = body(file, start, end - start + 1, content_type);
                *part.status_mut() = StatusCode::PARTIAL_CONTENT;
                insert(
                    part.headers_mut(),
                    header::CONTENT_RANGE,
                    &format!("bytes {start}-{end}/{size}"),
                );
                part
            }
            Range::Unsatisfiable => {
                let mut refused = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                insert(
                    refused.headers_mut(),
                    header::CONTENT_RANGE,
                    &format!("bytes */{size}"),
                );
                refused
            }
        }
    };
    let out = response.headers_mut();
    if let Some(date) = &last_modified {
        insert(out, header::LAST_MODIFIED, date);
    }
    shared_headers(out, &tag, "bytes");
    response
}

fn insert(headers: &mut HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// A strong validator from the file's size and modification time (DMN-253): hashing every byte of a video on each
/// request would cost more than sending it.
fn etag(size: u64, modified: Option<SystemTime>) -> String {
    let nanos = modified
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    format!("\"{size:x}-{nanos:x}\"")
}

/// Whether the client's copy is current: `If-None-Match` names the tag (or `*`), else, without `If-None-Match`,
/// `If-Modified-Since` is not before the modification time (to the second).
fn not_modified(headers: &HeaderMap, tag: &str, modified: Option<SystemTime>) -> bool {
    if let Some(tags) = header_text(headers, header::IF_NONE_MATCH) {
        return tags.split(',').map(str::trim).any(|candidate| {
            candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == tag
        });
    }
    let since = header_text(headers, header::IF_MODIFIED_SINCE)
        .and_then(|date| httpdate::parse_http_date(date).ok());
    match (since, modified) {
        (Some(since), Some(modified)) => {
            let seconds = |time: SystemTime| {
                time.duration_since(UNIX_EPOCH)
                    .map_or(0, |since| since.as_secs())
            };
            seconds(modified) <= seconds(since)
        }
        _ => false,
    }
}

/// Whether a `Range` applies: always without `If-Range`, else only when it names the current version (the strong tag
/// or the exact `Last-Modified` date); otherwise the whole file is sent.
fn if_range_holds(headers: &HeaderMap, tag: &str, last_modified: Option<&str>) -> bool {
    match header_text(headers, header::IF_RANGE) {
        None => true,
        Some(value) if value.starts_with('"') => value == tag,
        Some(value) => Some(value) == last_modified,
    }
}

/// What a `Range` header asks of a file of `size` bytes.
#[derive(Debug, PartialEq, Eq)]
enum Range {
    /// No usable range (several, or not `bytes=` syntax): the whole file.
    Whole,
    /// Bytes `start..=end`.
    Part {
        start: u64,
        end: u64,
    },
    Unsatisfiable,
}

/// One `bytes=` range (DMN-252): `a-b` (`b` capped at the last byte), `a-` or the suffix `-n`. A range starting past
/// the end, or a suffix of nothing, is unsatisfiable; anything else, several ranges included, is the whole file.
fn range(value: &str, size: u64) -> Range {
    let Some(spec) = value.trim().strip_prefix("bytes=") else {
        return Range::Whole;
    };
    let Some((first, last)) = spec.trim().split_once('-') else {
        return Range::Whole;
    };
    if spec.contains(',') {
        return Range::Whole;
    }
    let number = |text: &str| text.trim().parse::<u64>().ok();
    match (first.trim().is_empty(), number(first), number(last)) {
        (true, _, Some(suffix)) => {
            if suffix == 0 || size == 0 {
                Range::Unsatisfiable
            } else {
                Range::Part {
                    start: size.saturating_sub(suffix),
                    end: size - 1,
                }
            }
        }
        (false, Some(start), end)
            if last.trim().is_empty() || end.is_some_and(|end| end >= start) =>
        {
            if start >= size {
                Range::Unsatisfiable
            } else {
                Range::Part {
                    start,
                    end: end.map_or(size - 1, |end| end.min(size - 1)),
                }
            }
        }
        _ => Range::Whole,
    }
}

/// A `200` streaming `length` bytes of `file` from `start`, read a chunk at a time off the async threads, never held
/// whole (a video may be gigabytes).
fn body(file: File, start: u64, length: u64, content_type: &'static str) -> Response {
    let chunks = futures::stream::unfold(Some((file, start, length)), |state| async move {
        let (mut file, offset, left) = state?;
        if left == 0 {
            return None;
        }
        let read = tokio::task::spawn_blocking(move || {
            file.seek(SeekFrom::Start(offset))?;
            let mut chunk = Vec::new();
            (&mut file).take(left.min(CHUNK)).read_to_end(&mut chunk)?;
            Ok::<_, std::io::Error>((file, chunk))
        })
        .await
        .map_err(std::io::Error::other)
        .and_then(|read| read);
        match read {
            // The file got shorter: the response ends early and the client sees a short body.
            Ok((_, chunk)) if chunk.is_empty() => None,
            Ok((file, chunk)) => {
                let sent = chunk.len() as u64;
                Some((Ok(chunk), Some((file, offset + sent, left - sent))))
            }
            Err(error) => Some((Err(error), None)),
        }
    });
    let mut response = Body::from_stream(chunks).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    response
}

/// The `Content-Type` of a file (DMN-251): by its extension, else by its first bytes, else
/// `application/octet-stream`.
fn content_type(path: &Path, start: &[u8]) -> &'static str {
    by_extension(path)
        .or_else(|| sniff(start))
        .unwrap_or("application/octet-stream")
}

/// Media, documents and data by extension (case-insensitive); images as `workspaces.read_file` names them.
fn by_extension(path: &Path) -> Option<&'static str> {
    if let Some(mime) = image_mime(path) {
        return Some(mime);
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "csv" => "text/csv; charset=utf-8",
        _ => return None,
    })
}

/// The type a file's first bytes show: a few magic numbers, then UTF-8 text without NUL as plain text (HTML
/// included: it is shown, never rendered).
fn sniff(start: &[u8]) -> Option<&'static str> {
    const MAGIC: [(usize, &[u8], &str); 12] = [
        (0, b"%PDF-", "application/pdf"),
        (0, b"\x89PNG\r\n\x1a\n", "image/png"),
        (0, b"\xff\xd8\xff", "image/jpeg"),
        (0, b"GIF87a", "image/gif"),
        (0, b"GIF89a", "image/gif"),
        (8, b"WEBP", "image/webp"),
        (8, b"WAVE", "audio/wav"),
        (0, b"OggS", "audio/ogg"),
        (0, b"fLaC", "audio/flac"),
        (0, b"ID3", "audio/mpeg"),
        (4, b"ftyp", "video/mp4"),
        (0, b"\x1a\x45\xdf\xa3", "video/webm"),
    ];
    let riff = start.starts_with(b"RIFF");
    if let Some((_, _, mime)) = MAGIC.iter().find(|(at, magic, _)| {
        (*at != 8 || riff) && start.get(*at..at + magic.len()) == Some(*magic)
    }) {
        return Some(mime);
    }
    let text = match std::str::from_utf8(start) {
        Ok(_) => true,
        // A character cut by the end of the sniffed bytes.
        Err(error) => error.error_len().is_none(),
    };
    (text && !start.contains(&0)).then_some("text/plain; charset=utf-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-252
    #[test]
    fn a_single_byte_range_is_a_part_and_anything_else_the_whole_file() {
        let part = |start, end| Range::Part { start, end };
        assert_eq!(range("bytes=0-3", 10), part(0, 3));
        assert_eq!(range("bytes=4-", 10), part(4, 9));
        assert_eq!(range("bytes=8-100", 10), part(8, 9));
        assert_eq!(range("bytes=-3", 10), part(7, 9));
        assert_eq!(range("bytes=-30", 10), part(0, 9));
        assert_eq!(range("bytes=10-", 10), Range::Unsatisfiable);
        assert_eq!(range("bytes=-0", 10), Range::Unsatisfiable);
        assert_eq!(range("bytes=0-", 0), Range::Unsatisfiable);
        for whole in [
            "bytes=0-1,4-5",
            "bytes=5-2",
            "items=0-1",
            "bytes=a-b",
            "bytes=",
            "bytes=-",
        ] {
            assert_eq!(range(whole, 10), Range::Whole, "{whole}");
        }
    }

    // spec: DMN-251
    #[test]
    fn the_type_comes_from_the_extension_then_the_first_bytes() {
        let of = |name: &str, start: &[u8]| content_type(Path::new(name), start);
        assert_eq!(of("clip.MP4", b""), "video/mp4");
        assert_eq!(of("song.flac", b""), "audio/flac");
        assert_eq!(of("doc.pdf", b""), "application/pdf");
        assert_eq!(of("pic.png", b"not a png"), "image/png");
        assert_eq!(of("noext", b"%PDF-1.4\n"), "application/pdf");
        assert_eq!(of("sound", b"RIFF\x24\0\0\0WAVEfmt "), "audio/wav");
        assert_eq!(of("movie", b"\0\0\0\x18ftypmp42"), "video/mp4");
        assert_eq!(
            of("page.html", b"<script>x</script>"),
            "text/plain; charset=utf-8"
        );
        // A character cut at the end of the sniffed bytes is still text.
        assert_eq!(
            of("notes", &"é".as_bytes()[..1]),
            "text/plain; charset=utf-8"
        );
        assert_eq!(of("blob", b"a\0b"), "application/octet-stream");
    }
}
