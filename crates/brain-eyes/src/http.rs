//! The server: loopback, read-only, no mutation endpoint.
//!
//! A small pool of worker threads shares one held graph view, so a slow
//! body read cannot block the tab next to it. Requests that ask for
//! something the graph does not have get a 404 or 400 and a sentence —
//! never a 500 and a stack of jargon.

use crate::body;
use crate::query;
use crate::state::{AppState, Config};
use brain_core::ids::StableId;
use serde::Serialize;
use std::sync::Arc;
use std::thread;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const INDEX_HTML: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const MRI_JS: &str = include_str!("../assets/mri.js");
const STYLES_CSS: &str = include_str!("../assets/styles.css");
const WORKERS: usize = 4;

pub fn serve(config: Config) -> Result<(), String> {
    let (server, address, state) = bind(config)?;
    let snapshot = state.snapshot()?;
    println!("Eyes: http://{address}");
    println!(
        "watching {} · {} objects · read-only",
        snapshot.prefix, snapshot.objects
    );
    run(server, state)
}

/// Bind the listener and warm the graph view. Split from [`serve`] so a
/// test can hold the address and drive real requests.
pub fn bind(config: Config) -> Result<(Arc<Server>, String, Arc<AppState>), String> {
    let bind = format!("{}:{}", config.bind, config.port);
    let server = Server::http(&bind).map_err(|e| format!("cannot listen on {bind}: {e}"))?;
    let address = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.to_string())
        .unwrap_or(bind);
    let state = Arc::new(AppState::new(config)?);
    Ok((Arc::new(server), address, state))
}

pub fn run(server: Arc<Server>, state: Arc<AppState>) -> Result<(), String> {
    for _ in 0..WORKERS.saturating_sub(1) {
        let server = Arc::clone(&server);
        let state = Arc::clone(&state);
        thread::spawn(move || loop {
            match server.recv() {
                Ok(request) => dispatch(request, &state),
                Err(_) => break,
            }
        });
    }
    loop {
        match server.recv() {
            Ok(request) => dispatch(request, &state),
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// One request, isolated: a panic in a handler must not take the server
/// down with it.
fn dispatch(request: Request, state: &Arc<AppState>) {
    let url = request.url().to_string();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle(request, state);
    }));
    if outcome.is_err() {
        eprintln!("eyes: a request for {url} failed unexpectedly (the server is still running)");
    }
}

fn handle(request: Request, state: &Arc<AppState>) {
    if request.method() != &Method::Get {
        return error(request, 405, "Eyes only answers GET — it never writes.");
    }
    let (path, query_string) = split_url(request.url());
    let path = path.to_string();
    let query_string = query_string.to_string();
    let param = |key: &str| query_param(&query_string, key).map(percent_decode);

    match path.as_str() {
        "/" | "/index.html" => text(request, 200, INDEX_HTML, "text/html; charset=utf-8"),
        "/assets/app.js" => text(request, 200, APP_JS, "text/javascript; charset=utf-8"),
        "/assets/mri.js" => text(request, 200, MRI_JS, "text/javascript; charset=utf-8"),
        "/assets/styles.css" => text(request, 200, STYLES_CSS, "text/css; charset=utf-8"),

        "/api/snapshot" => json_result(request, state.snapshot()),
        "/api/now" => json_result(request, state.read(query::now::build)),
        "/api/timeline" => {
            let limit = param("limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(40usize)
                .clamp(1, 200);
            json_result(request, state.read(|loaded| query::timeline::build(loaded, limit)))
        }
        "/api/library" => {
            let shelf = param("shelf").unwrap_or_default();
            let text_query = param("q").unwrap_or_default();
            json_result(
                request,
                state.read(|loaded| query::library::build(loaded, &shelf, &text_query)),
            )
        }
        "/api/concepts" => json_result(request, state.read(query::library::concepts)),
        "/api/tests" => json_result(request, state.read(query::tests::build)),
        "/api/work" => json_result(request, state.read(query::work::build)),
        "/api/mri" => json_result(request, state.read(|loaded| loaded.mri())),
        "/api/evidence" => json_result(request, state.read(query::evidence::build)),
        "/api/media" => {
            let root = state.config.content_root.clone();
            json_result(
                request,
                state.read(|loaded| query::media::build(loaded, Some(&root))),
            )
        }
        "/api/map" => {
            let lens = param("lens").unwrap_or_else(|| "attention".to_string());
            json_result(request, state.read(|loaded| query::map::build(loaded, &lens)))
        }
        "/api/thing" => {
            let Some(id) = param("id") else {
                return error(request, 400, "Which thing? Pass ?id=…");
            };
            let root = state.config.content_root.clone();
            match state.read(|loaded| query::thing::build(loaded, &id, Some(&root))) {
                Ok(view) => json(request, &view),
                Err(message) => error(request, 404, &message),
            }
        }
        "/api/find" => {
            let text_query = param("q").unwrap_or_default();
            let limit = param("limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(20usize)
                .clamp(1, 100);
            json_result(
                request,
                state.read(|loaded| query::find::build(loaded, &text_query, limit)),
            )
        }
        "/api/body" => {
            let Some(id) = param("id") else {
                return error(request, 400, "Which body? Pass ?id=…");
            };
            raw_body(request, state, &id)
        }
        _ => error(request, 404, "Eyes has nothing at that address."),
    }
}

fn raw_body(request: Request, state: &Arc<AppState>, id: &str) {
    let root = state.config.content_root.clone();
    let resolved = state.read(|loaded| {
        let sid = StableId(id.to_string());
        let kind = query::kind_of(&loaded.index, &loaded.store, &sid)
            .ok_or_else(|| "this entity is not in the current graph".to_string())?;
        let labels = query::labels_of(&loaded.index, &loaded.store, &sid);
        let resolved = body::resolve(loaded, &sid, &kind, &labels, Some(&root))?;
        Ok((resolved.view, resolved.bytes))
    });
    let (view, bytes) = match resolved {
        Ok(pair) => pair,
        Err(message) => return error(request, 400, &message),
    };

    // Only media keeps its real type; everything else is served as
    // plain text so nothing from the workspace can execute.
    let media = matches!(view.format.as_str(), "image" | "audio" | "video" | "pdf");
    let content_type = if media {
        view.media_type.as_str()
    } else {
        "text/plain; charset=utf-8"
    };
    let filename = view
        .path
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("body")
        .replace(['"', '\r', '\n'], "_");
    let disposition = if media {
        format!("inline; filename=\"{filename}\"")
    } else {
        format!("attachment; filename=\"{filename}\"")
    };

    // A <video> or <audio> element cannot seek without byte ranges, and
    // Safari will not start playback at all without them. The tour and
    // every Playwright recording depend on this.
    let total = bytes.len() as u64;
    let requested = header_value(&request, "range").and_then(|value| parse_range(&value, total));
    let (status, bytes, content_range) = match requested {
        Some(Ok((start, end))) => {
            let slice = bytes[start as usize..=end as usize].to_vec();
            (
                206,
                slice,
                Some(format!("bytes {start}-{end}/{total}")),
            )
        }
        // A range outside the file is a 416, not a silent whole-file reply.
        Some(Err(())) => {
            let mut response = Response::from_data(Vec::new()).with_status_code(StatusCode(416));
            for (name, value) in [
                ("Content-Range", format!("bytes */{total}")),
                ("Accept-Ranges", "bytes".to_string()),
            ] {
                if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
                    response = response.with_header(header);
                }
            }
            return drop(request.respond(response));
        }
        None => (200, bytes, None),
    };

    let mut response = Response::from_data(bytes)
        .with_status_code(StatusCode(status))
        .with_chunked_threshold(NEVER_CHUNK);
    let mut headers: Vec<(&str, String)> = vec![
        ("Content-Type", content_type.to_string()),
        ("Content-Disposition", disposition),
        ("Accept-Ranges", "bytes".to_string()),
        ("X-Content-Type-Options", "nosniff".to_string()),
        // Media is fetched a range at a time while scrubbing; refusing to
        // store it means re-reading the whole file on every seek. The
        // window is short and the server is loopback-only.
        (
            "Cache-Control",
            if media {
                "private, max-age=60".to_string()
            } else {
                "no-store".to_string()
            },
        ),
        ("Content-Security-Policy", "default-src 'none'; sandbox".to_string()),
    ];
    if let Some(range) = content_range {
        headers.push(("Content-Range", range));
    }
    for (name, value) in headers {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }
    let _ = request.respond(response);
}

fn header_value(request: &Request, field: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(field))
        .map(|header| header.value.as_str().to_string())
}

/// Parse a single byte range against a known length.
///
/// `Ok((start, end))` is inclusive on both ends, as HTTP defines it.
/// `Err(())` means the range is unsatisfiable; `None` means there was no
/// usable range and the whole body should be sent. Multi-range requests
/// fall into `None`: no browser needs them for media, and answering one
/// range of several would be a lie.
pub fn parse_range(value: &str, total: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = value.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (from, to) = spec.split_once('-')?;
    if total == 0 {
        return Some(Err(()));
    }
    let last = total - 1;
    let (start, end) = match (from.trim(), to.trim()) {
        // `-500`: the final 500 bytes.
        ("", suffix) => {
            let length: u64 = suffix.parse().ok()?;
            if length == 0 {
                return Some(Err(()));
            }
            (total.saturating_sub(length), last)
        }
        (start, "") => (start.parse().ok()?, last),
        (start, end) => (start.parse().ok()?, end.parse::<u64>().ok()?.min(last)),
    };
    if start > last || start > end {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
}

fn json_result<T: Serialize>(request: Request, result: Result<T, String>) {
    match result {
        Ok(value) => json(request, &value),
        Err(message) => error(request, 500, &message),
    }
}

fn json<T: Serialize>(request: Request, value: &T) {
    match serde_json::to_string(value) {
        Ok(body) => text(request, 200, &body, "application/json; charset=utf-8"),
        Err(error_value) => error(request, 500, &error_value.to_string()),
    }
}

fn error(request: Request, status: u16, message: &str) {
    let body = serde_json::json!({ "error": message }).to_string();
    text(request, status, &body, "application/json; charset=utf-8");
}

/// Above this many bytes tiny_http would switch to chunked transfer.
///
/// Every response here is already complete in memory, so a length is
/// always knowable and always better: chunked cost the browser a flat
/// twenty seconds per large response while `curl` saw two milliseconds.
const NEVER_CHUNK: usize = usize::MAX;

fn text(request: Request, status: u16, body: &str, content_type: &str) {
    let mut response = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_chunked_threshold(NEVER_CHUNK);
    for (name, value) in [
        ("Content-Type", content_type),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
        (
            "Content-Security-Policy",
            "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; img-src 'self' data:",
        ),
    ] {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }
    let _ = request.respond(response);
}

pub fn split_url(url: &str) -> (&str, &str) {
    url.split_once('?').unwrap_or((url, ""))
}

pub fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|part| {
        let (candidate, value) = part.split_once('=')?;
        (percent_decode(candidate) == key).then_some(value)
    })
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
            }
            b'+' => out.push(b' '),
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
