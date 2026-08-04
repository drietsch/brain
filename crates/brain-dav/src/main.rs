//! `brain-dav` — serve the graph as a read-only WebDAV volume.
//!
//! ```text
//! brain-dav [--prefix twin/self] [--port 4918] [--dir .]
//! mount_webdav -S http://127.0.0.1:4918/ /Volumes/brain   # macOS
//! ```
//!
//! It binds to loopback like the cockpit does, and it answers no verb
//! that could change anything.

use std::sync::Arc;

use brain_eyes::{AppState, Config};
use dav_server::{fakels::FakeLs, DavHandler, DavMethod, DavMethodSet};
use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

/// GET, HEAD, OPTIONS, PROPFIND — plus LOCK and UNLOCK, which exist only
/// so macOS and Windows will mount the volume at all. Nothing here can
/// change anything.
fn read_only_methods() -> DavMethodSet {
    let mut set = DavMethodSet::WEBDAV_RO;
    set.add(DavMethod::Lock);
    set.add(DavMethod::Unlock);
    set
}

/// The verbs this surface answers. Everything else is refused with the
/// route that would have worked.
const READS: [&str; 6] = ["GET", "HEAD", "OPTIONS", "PROPFIND", "LOCK", "UNLOCK"];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::default();
    let mut port = 4918u16;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--prefix" => config.prefix = it.next().cloned().unwrap_or(config.prefix),
            "--dir" => config.content_root = it.next().map(Into::into).unwrap_or(config.content_root),
            "--port" => port = it.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            other => return Err(format!("unexpected argument '{other}'").into()),
        }
    }

    let state = Arc::new(AppState::new(config)?);
    let handler = DavHandler::builder()
        .filesystem(brain_dav::GraphFs::new(state.clone()))
        // GET, HEAD, OPTIONS, PROPFIND — and LOCK, only so the volume can
        // be mounted at all. Every other verb is refused before it reaches
        // the filesystem, and `Allow:` says so rather than advertising a
        // COPY that would fail: a surface that offers what it will not do
        // is the same lie as a document that has drifted from its code.
        .methods(read_only_methods())
        .locksystem(FakeLs::new())
        .build_handler();

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    println!("brain-dav: http://{addr}/  (read only)");
    println!("  mount_webdav -S http://{addr}/ /Volumes/brain");

    loop {
        let (stream, _) = listener.accept().await?;
        let handler = handler.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let handler = handler.clone();
                let state = state.clone();
                async move {
                    // A refusal that only says "no" teaches nothing. Answer
                    // the write verbs here, with the command that would
                    // have done what the caller wanted.
                    if !READS.contains(&req.method().as_str()) {
                        let body = brain_dav::refusal(
                            &state,
                            req.method().as_str(),
                            req.uri().path(),
                        );
                        let response = hyper::Response::builder()
                            .status(hyper::StatusCode::METHOD_NOT_ALLOWED)
                            .header("Allow", "OPTIONS, HEAD, GET, PROPFIND, LOCK, UNLOCK")
                            .header("Content-Type", "text/plain; charset=utf-8")
                            .body(dav_server::body::Body::from(body))
                            .expect("static response");
                        return Ok::<_, std::convert::Infallible>(response);
                    }
                    Ok::<_, std::convert::Infallible>(handler.handle(req).await)
                }
            });
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}
