//! Localhost server for the CodeMirror preview.
//!
//! This is not [`crate::preview_server`]. That one stays on
//! `127.0.0.1:39473` for the theme and feedback panels. This one binds
//! `127.0.0.1:0`, requires the per-process token, and never logs it.
//!
//! Step ① of `docs/codemirror-preview-design.md`. The shell's Read path
//! calls [`read_preview`]. The macOS host starts one shared listener from
//! [`with_shared`] the first time a WebView Read needs it.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};

const INDEX_HTML: &str = include_str!("../../../experiments/preview-webview/index.html");
const BUNDLE_JS: &[u8] = include_bytes!("../../../experiments/preview-webview/bundle.js");

const HEADER_LIMIT: usize = 16 * 1024;
const EVENT_BODY_LIMIT: usize = 64 * 1024;
const MAX_CONNECTIONS: usize = 32;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) use crate::preview_session::{
    apply_event, finish_read, random_16, read_opened, Cache, PreviewSession, SessionView, MAX_DOCS,
};
pub use crate::preview_session::{
    file_identity, read_preview, FileId, PreviewEvent, PreviewText, ReadPreviewError, PREVIEW_CAP,
};

struct Limits {
    max_connections: usize,
    read_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: MAX_CONNECTIONS,
            read_timeout: READ_TIMEOUT,
        }
    }
}

type NotifyFn = Arc<dyn Fn() + Send + Sync>;

struct Ctx {
    token: [u8; 16],
    port: u16,
    session: Arc<Mutex<PreviewSession>>,
    events: Arc<Mutex<Vec<PreviewEvent>>>,
    notify: Arc<Mutex<Option<NotifyFn>>>,
}

pub struct PreviewWeb {
    port: u16,
    token_hex: String,
    inner: Arc<Mutex<PreviewSession>>,
    events: Arc<Mutex<Vec<PreviewEvent>>>,
    notify: Arc<Mutex<Option<NotifyFn>>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PreviewWeb {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token(&self) -> &str {
        &self.token_hex
    }

    pub fn preview_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/preview?token={}",
            self.port, self.token_hex
        )
    }

    pub fn with_session<T>(&self, f: impl FnOnce(&mut PreviewSession) -> T) -> T {
        let mut session = self.inner.lock().expect("preview session");
        f(&mut session)
    }

    pub fn drain_events(&self) -> Vec<PreviewEvent> {
        std::mem::take(&mut *self.events.lock().expect("preview events"))
    }

    pub(crate) fn set_notifier(&self, notify: NotifyFn) {
        if let Ok(mut slot) = self.notify.lock() {
            *slot = Some(notify);
        }
    }

    pub(crate) fn session_view(&self) -> SessionView {
        self.with_session(|session| session.view())
    }

    pub(crate) fn ready_text(&self, id: [u8; 16]) -> Option<(String, bool)> {
        self.with_session(|session| session.ready_text(id))
    }

    /// Main-thread registration. The disk read runs on another thread and
    /// bumps [`PreviewSession::version`] only after it finishes.
    pub fn register(&self, path: &Path) -> std::io::Result<[u8; 16]> {
        let canonical = path.canonicalize()?;
        let file = File::open(&canonical)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a file",
            ));
        }
        let file_id = file_identity(&file)?;
        let name = canonical
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| canonical.display().to_string());
        let (id, generation) =
            self.with_session(|session| session.begin_doc(canonical.clone(), file_id, name));
        let inner = Arc::clone(&self.inner);
        let notify = Arc::clone(&self.notify);
        thread::spawn(move || {
            let opened = File::open(&canonical);
            let (observed, result) = match opened {
                Ok(file) => {
                    let observed = file_identity(&file).ok();
                    (observed, read_opened(file, PREVIEW_CAP))
                }
                Err(err) => (None, Err(ReadPreviewError::Io(err))),
            };
            {
                let mut session = inner.lock().expect("preview session");
                finish_read(&mut session, id, generation, observed, result);
            }
            fire_notify(&notify);
        });
        Ok(id)
    }
}

impl Drop for PreviewWeb {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start() -> std::io::Result<PreviewWeb> {
    start_with(Limits::default())
}

fn start_with(limits: Limits) -> std::io::Result<PreviewWeb> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let token = random_16()?;
    let notify = Arc::new(Mutex::new(None));
    let ctx = Arc::new(Ctx {
        token,
        port,
        session: Arc::new(Mutex::new(PreviewSession::new())),
        events: Arc::new(Mutex::new(Vec::new())),
        notify: Arc::clone(&notify),
    });
    let shutdown = Arc::new(AtomicBool::new(false));
    let inflight = Arc::new(AtomicUsize::new(0));
    let accept_shutdown = Arc::clone(&shutdown);
    let accept_ctx = Arc::clone(&ctx);
    let thread =
        thread::spawn(move || accept_loop(listener, accept_ctx, limits, inflight, accept_shutdown));
    Ok(PreviewWeb {
        port,
        token_hex: encode_hex(&token),
        inner: Arc::clone(&ctx.session),
        events: Arc::clone(&ctx.events),
        notify,
        shutdown,
        thread: Some(thread),
    })
}

fn accept_loop(
    listener: TcpListener,
    ctx: Arc<Ctx>,
    limits: Limits,
    inflight: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if inflight.load(Ordering::SeqCst) >= limits.max_connections {
                    let _ = write_response(&mut stream, too_many());
                    continue;
                }
                inflight.fetch_add(1, Ordering::SeqCst);
                let inflight = Arc::clone(&inflight);
                let ctx = Arc::clone(&ctx);
                let timeout = limits.read_timeout;
                thread::spawn(move || {
                    struct Guard(Arc<AtomicUsize>);
                    impl Drop for Guard {
                        fn drop(&mut self) {
                            self.0.fetch_sub(1, Ordering::SeqCst);
                        }
                    }
                    let _guard = Guard(inflight);
                    let _ = serve_connection(stream, &ctx, timeout);
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

fn serve_connection(mut stream: TcpStream, ctx: &Ctx, timeout: Duration) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(timeout))?;
    let raw = match read_request(&mut stream) {
        Ok(raw) => raw,
        Err(ReadFail::Timeout) => return Ok(()),
        Err(ReadFail::Io(err)) => {
            let _ = err;
            return Ok(());
        }
        Err(ReadFail::HeaderTooLarge) => {
            return write_response(&mut stream, text(400, "header too large"));
        }
        Err(ReadFail::BodyTooLarge) => {
            return write_response(&mut stream, text(413, "body too large"));
        }
    };
    write_response(&mut stream, handle(ctx, &raw))
}

struct RawRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

enum ReadFail {
    Timeout,
    Io(std::io::Error),
    HeaderTooLarge,
    BodyTooLarge,
}

fn read_request(stream: &mut TcpStream) -> Result<RawRequest, ReadFail> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    let header_end = loop {
        if buf.len() > HEADER_LIMIT {
            return Err(ReadFail::HeaderTooLarge);
        }
        let n = match stream.read(&mut tmp) {
            Ok(0) => {
                return Err(ReadFail::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "eof",
                )))
            }
            Ok(n) => n,
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock =>
            {
                return Err(ReadFail::Timeout);
            }
            Err(err) => return Err(ReadFail::Io(err)),
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/");
    let (path, query_str) = target.split_once('?').unwrap_or((target, ""));
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let mut body = buf[header_end + 4..].to_vec();
    if method == "POST" {
        let Some(length) = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
        else {
            return Ok(RawRequest {
                method,
                path: path.to_string(),
                query: parse_query(query_str),
                headers,
                body,
            });
        };
        if length > EVENT_BODY_LIMIT {
            return Err(ReadFail::BodyTooLarge);
        }
        while body.len() < length {
            let n = match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => n,
                Err(err)
                    if err.kind() == std::io::ErrorKind::TimedOut
                        || err.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    return Err(ReadFail::Timeout);
                }
                Err(err) => return Err(ReadFail::Io(err)),
            };
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(length);
    }
    Ok(RawRequest {
        method,
        path: path.to_string(),
        query: parse_query(query_str),
        headers,
        body,
    })
}

fn handle(ctx: &Ctx, raw: &RawRequest) -> Response {
    let host = raw.headers.get("host").map(String::as_str).unwrap_or("");
    if host != format!("127.0.0.1:{}", ctx.port) && host != format!("[::1]:{}", ctx.port) {
        return text(400, "bad host");
    }
    let token = raw.query.get("token").map(String::as_str).unwrap_or("");
    let Some(token) = decode_hex_16(token) else {
        return text(404, "not found");
    };
    if !same_bytes(&token, &ctx.token) {
        return text(404, "not found");
    }
    match (raw.method.as_str(), raw.path.as_str()) {
        ("GET", "/preview") => preview_page(&ctx.token),
        ("GET", "/bundle.js") => bytes(200, "application/javascript", BUNDLE_JS.to_vec(), vec![]),
        ("GET", "/session") => session_response(ctx, raw.query.get("since").map(String::as_str)),
        ("GET", "/file") => {
            file_response(ctx, raw.query.get("id").map(String::as_str).unwrap_or(""))
        }
        ("POST", "/event") => event_response(ctx, raw),
        (_, "/preview" | "/bundle.js" | "/session" | "/file" | "/event") => {
            text(405, "method not allowed")
        }
        _ => text(404, "not found"),
    }
}

fn preview_page(token: &[u8; 16]) -> Response {
    const NEEDLE: &str = "src=\"bundle.js\"";
    assert_eq!(
        INDEX_HTML.matches(NEEDLE).count(),
        1,
        "preview page must reference bundle.js exactly once"
    );
    let html = INDEX_HTML.replace(
        NEEDLE,
        &format!("src=\"/bundle.js?token={}\"", encode_hex(token)),
    );
    bytes(
        200,
        "text/html; charset=utf-8",
        html.into_bytes(),
        vec![(
            "Content-Security-Policy".to_string(),
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:".to_string(),
        )],
    )
}

fn session_response(ctx: &Ctx, since: Option<&str>) -> Response {
    let session = ctx.session.lock().expect("preview session");
    if let Some(since) = since {
        if since.is_empty() {
            // First poll omits a known version.
        } else {
            let Ok(since) = since.parse::<u64>() else {
                return text(400, "bad since");
            };
            if since == session.version {
                return Response {
                    status: 204,
                    headers: vec![],
                    body: Vec::new(),
                };
            }
        }
    }
    let open_docs: Vec<Value> = session
        .order
        .iter()
        .filter_map(|id| {
            let doc = session.docs.get(id)?;
            if matches!(doc.cache, Cache::Pending) {
                return None;
            }
            Some(json!({
                "id": encode_hex(id),
                "name": doc.name,
                "rev": doc.generation,
            }))
        })
        .collect();
    let active = session.active.and_then(|id| {
        let doc = session.docs.get(&id)?;
        if matches!(doc.cache, Cache::Pending) {
            None
        } else {
            Some(encode_hex(&id))
        }
    });
    let body = json!({
        "version": session.version,
        "open-docs": open_docs,
        "active": active,
        "theme": { "dark": session.theme_dark },
    });
    bytes(
        200,
        "application/json",
        body.to_string().into_bytes(),
        vec![],
    )
}

fn file_response(ctx: &Ctx, id: &str) -> Response {
    let Some(id) = decode_hex_16(id) else {
        return text(404, "not found");
    };
    let session = ctx.session.lock().expect("preview session");
    let Some(doc) = session.docs.get(&id) else {
        return text(404, "not found");
    };
    match &doc.cache {
        Cache::Pending => {
            let mut response = text(503, "not ready");
            response
                .headers
                .push(("Retry-After".to_string(), "1".to_string()));
            response
        }
        Cache::Failed { status, message } => text(*status, message),
        Cache::Ready {
            bytes: file_bytes,
            truncated,
        } => {
            let mut extra = vec![("X-Content-Type-Options".to_string(), "nosniff".to_string())];
            if *truncated {
                extra.push(("X-Preview-Truncated".to_string(), "1".to_string()));
            }
            bytes(200, "text/plain; charset=utf-8", file_bytes.clone(), extra)
        }
    }
}

fn event_response(ctx: &Ctx, raw: &RawRequest) -> Response {
    if !raw.headers.contains_key("content-length") {
        return text(411, "length required");
    }
    let content_type = raw
        .headers
        .get("content-type")
        .map(String::as_str)
        .unwrap_or("");
    if !content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/json")
    {
        return text(415, "expected application/json");
    }
    if let Some(origin) = raw.headers.get("origin") {
        let own = format!("http://127.0.0.1:{}", ctx.port);
        let own_v6 = format!("http://[::1]:{}", ctx.port);
        if origin != &own && origin != &own_v6 {
            return text(403, "bad origin");
        }
    }
    if raw
        .headers
        .get("sec-fetch-site")
        .is_some_and(|value| value.eq_ignore_ascii_case("cross-site"))
    {
        return text(403, "cross-site");
    }
    let Ok(value) = serde_json::from_slice::<Value>(&raw.body) else {
        return text(400, "bad json");
    };
    let Some(event) = parse_event(&value) else {
        return text(400, "bad event");
    };
    {
        let mut session = ctx.session.lock().expect("preview session");
        apply_event(&mut session, &event);
    }
    ctx.events.lock().expect("preview events").push(event);
    fire_notify(&ctx.notify);
    text(204, "")
}

fn fire_notify(notify: &Mutex<Option<NotifyFn>>) {
    let callback = notify.lock().ok().and_then(|slot| slot.clone());
    if let Some(callback) = callback {
        callback();
    }
}

fn parse_event(value: &Value) -> Option<PreviewEvent> {
    let kind = value.get("type")?.as_str()?;
    match kind {
        "ready" => Some(PreviewEvent::Ready),
        "close" => Some(PreviewEvent::Close),
        "close-tab" => Some(PreviewEvent::CloseTab {
            id: decode_hex_16(value.get("id")?.as_str()?)?,
        }),
        "select-tab" => Some(PreviewEvent::SelectTab {
            id: decode_hex_16(value.get("id")?.as_str()?)?,
        }),
        "error" => Some(PreviewEvent::Error {
            message: value.get("message")?.as_str()?.to_string(),
        }),
        _ => None,
    }
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn text(status: u16, body: &str) -> Response {
    let content_type = if status == 204 {
        None
    } else {
        Some("text/plain; charset=utf-8")
    };
    bytes(
        status,
        content_type.unwrap_or(""),
        body.as_bytes().to_vec(),
        vec![],
    )
}

fn bytes(
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    mut headers: Vec<(String, String)>,
) -> Response {
    if !content_type.is_empty() {
        headers.insert(0, ("Content-Type".to_string(), content_type.to_string()));
    }
    headers.push(("Cache-Control".to_string(), "no-store".to_string()));
    Response {
        status,
        headers,
        body,
    }
}

fn too_many() -> Response {
    let mut response = text(503, "too many connections");
    response
        .headers
        .push(("Retry-After".to_string(), "1".to_string()));
    response
}

fn write_response(stream: &mut TcpStream, response: Response) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        response.status,
        reason(response.status)
    );
    for (name, value) in &response.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    if response.status != 204 {
        head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if response.status != 204 {
        stream.write_all(&response.body)?;
    }
    stream.flush()?;
    Ok(())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if query.is_empty() {
        return out;
    }
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        out.insert(percent_decode(key), percent_decode(value));
    }
    out
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            ) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn decode_hex_16(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for index in 0..16 {
        out[index] = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn same_bytes(left: &[u8; 16], right: &[u8; 16]) -> bool {
    let mut diff = 0u8;
    for index in 0..16 {
        diff |= left[index] ^ right[index];
    }
    diff == 0
}

static SHARED: Mutex<Option<PreviewWeb>> = Mutex::new(None);
static NOTIFIER: Mutex<Option<NotifyFn>> = Mutex::new(None);

/// Install the wake used by the shell. The callback must not lock the session.
pub(crate) fn set_shared_notifier(notify: NotifyFn) {
    if let Ok(mut slot) = NOTIFIER.lock() {
        *slot = Some(Arc::clone(&notify));
    }
    if let Ok(guard) = SHARED.lock() {
        if let Some(web) = guard.as_ref() {
            web.set_notifier(notify);
        }
    }
}

/// Start the process server on first use. A poisoned lock or a failed bind
/// returns `None` so the caller can fall back to the native view. The closure
/// must not call [`with_shared`] again.
pub(crate) fn with_shared<T>(f: impl FnOnce(&PreviewWeb) -> T) -> Option<T> {
    let mut guard = SHARED.lock().ok()?;
    if guard.is_none() {
        let notify = NOTIFIER.lock().ok().and_then(|slot| slot.clone());
        let Ok(web) = start() else {
            return None;
        };
        if let Some(notify) = notify {
            web.set_notifier(notify);
        }
        *guard = Some(web);
    }
    Some(f(guard.as_ref().expect("preview server was just stored")))
}

pub(crate) fn session_view_shared() -> Option<SessionView> {
    SHARED.lock().ok()?.as_ref().map(PreviewWeb::session_view)
}

pub(crate) fn ready_text_shared(id: [u8; 16]) -> Option<(String, bool)> {
    SHARED
        .lock()
        .ok()?
        .as_ref()
        .and_then(|web| web.ready_text(id))
}

pub(crate) fn shared_preview_url() -> Option<String> {
    SHARED.lock().ok()?.as_ref().map(PreviewWeb::preview_url)
}

pub(crate) fn set_theme_dark_shared(dark: bool) {
    let Ok(guard) = SHARED.lock() else {
        return;
    };
    let Some(web) = guard.as_ref() else {
        return;
    };
    web.with_session(|session| session.set_theme_dark(dark));
}

/// Drop the document stack. An empty session is left alone, so a second
/// call does not bump the version. This does not start the server.
pub(crate) fn clear_stack() {
    let Ok(guard) = SHARED.lock() else {
        return;
    };
    let Some(web) = guard.as_ref() else {
        return;
    };
    web.with_session(|session| session.clear());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eb-preview-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    fn wait_version(web: &PreviewWeb, id: [u8; 16], min: u64) {
        let start = Instant::now();
        loop {
            let ready = web.with_session(|session| {
                session.version >= min
                    && session
                        .docs
                        .get(&id)
                        .is_some_and(|doc| !matches!(doc.cache, Cache::Pending))
            });
            if ready {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "read did not finish"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    struct Http {
        status: u16,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    fn exchange(port: u16, request: &str) -> Http {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf);
        let sep = buf
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("response headers");
        let head = std::str::from_utf8(&buf[..sep]).unwrap();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let mut headers = HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        Http {
            status,
            headers,
            body: buf[sep + 4..].to_vec(),
        }
    }

    fn get(web: &PreviewWeb, path: &str) -> Http {
        exchange(
            web.port(),
            &format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                web.port()
            ),
        )
    }

    #[test]
    fn read_preview_cuts_on_a_char_boundary_and_rejects_invalid_utf8() {
        let dir = temp_dir("read");
        let cut = write_file(&dir, "cut.txt", "a你b".as_bytes());
        let text = read_preview(&cut, 3).unwrap();
        assert_eq!(text.as_str(), "a");
        assert!(text.truncated);

        let exact = write_file(&dir, "exact.txt", b"abcd");
        let text = read_preview(&exact, 4).unwrap();
        assert_eq!(text.as_str(), "abcd");
        assert!(!text.truncated);

        let over = write_file(&dir, "over.txt", b"abcde");
        let text = read_preview(&over, 4).unwrap();
        assert_eq!(text.as_str(), "abcd");
        assert!(text.truncated);

        let binary = write_file(&dir, "bin.txt", &[b'a', 0xff, b'b', b'c']);
        assert!(matches!(
            read_preview(&binary, 3),
            Err(ReadPreviewError::NotUtf8)
        ));
        assert!(matches!(
            read_preview(&binary, 8),
            Err(ReadPreviewError::NotUtf8)
        ));

        let mut big = vec![b'a'; PREVIEW_CAP + 1];
        big[PREVIEW_CAP] = b'b';
        let huge = write_file(&dir, "huge.txt", &big);
        let text = read_preview(&huge, PREVIEW_CAP).unwrap();
        assert_eq!(text.bytes.len(), PREVIEW_CAP);
        assert!(text.truncated);
        assert_eq!(PREVIEW_CAP, 5 * 1024 * 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn routes_require_token_and_host_and_serve_the_page() {
        let web = start().unwrap();
        let missing = get(&web, "/preview");
        assert_eq!(missing.status, 404);

        let bad_host = exchange(
            web.port(),
            &format!(
                "GET /preview?token={} HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n",
                web.token(),
                web.port()
            ),
        );
        assert_eq!(bad_host.status, 400);

        let page = get(&web, &format!("/preview?token={}", web.token()));
        assert_eq!(page.status, 200);
        let html = String::from_utf8(page.body).unwrap();
        assert!(html.contains(&format!("/bundle.js?token={}", web.token())));
        assert!(!html.contains("src=\"bundle.js\""));
        assert!(html.contains("white-space: nowrap"));
        assert!(html.contains("tab-name"));
        let csp = page.headers.get("content-security-policy").unwrap();
        assert!(csp.contains("script-src 'self';"));
        assert!(!csp
            .split(';')
            .any(|part| part.contains("script-src") && part.contains("unsafe-inline")));

        let script = get(&web, &format!("/bundle.js?token={}", web.token()));
        assert_eq!(script.status, 200);
        assert_eq!(script.headers["content-type"], "application/javascript");
        assert!(!script.body.is_empty());
        let script_text = std::str::from_utf8(&script.body).unwrap();
        assert!(!script_text.contains("/Users/"));
        assert!(script_text.contains("__ebPreviewWake"));
        assert!(script_text.contains("tab-name"));
        assert!(script_text.contains("activeRev"));

        assert_eq!(
            crate::preview_server::url(),
            "http://127.0.0.1:39473/index.html"
        );
        assert_eq!(
            crate::preview_server::theme_url(),
            "http://127.0.0.1:39473/theme/index.html"
        );
    }

    #[test]
    fn session_file_reread_clear_and_evict_bump() {
        let web = start().unwrap();
        let dir = temp_dir("session");
        let path = write_file(&dir, "notes.md", b"hello");
        let id = web.register(&path).unwrap();
        wait_version(&web, id, 1);
        let first = web.with_session(|session| session.version());
        let session = get(&web, &format!("/session?token={}&since=0", web.token()));
        assert_eq!(session.status, 200);
        let body: Value = serde_json::from_slice(&session.body).unwrap();
        assert_eq!(body["version"], first);
        assert_eq!(body["open-docs"][0]["id"], encode_hex(&id));
        assert_eq!(body["open-docs"][0]["name"], "notes.md");
        assert_eq!(body["open-docs"][0]["rev"], 1);
        assert_eq!(body["active"], encode_hex(&id));
        assert_eq!(body["theme"]["dark"], false);

        let same = get(
            &web,
            &format!("/session?token={}&since={first}", web.token()),
        );
        assert_eq!(same.status, 204);
        assert!(same.body.is_empty());

        let file = get(
            &web,
            &format!("/file?token={}&id={}", web.token(), encode_hex(&id)),
        );
        assert_eq!(file.status, 200);
        assert_eq!(file.body, b"hello");
        assert_eq!(file.headers["content-type"], "text/plain; charset=utf-8");
        assert_eq!(file.headers["x-content-type-options"], "nosniff");
        assert!(file.headers.get("x-preview-truncated").is_none());

        write_file(&dir, "notes.md", b"world");
        let again = web.register(&path).unwrap();
        assert_eq!(again, id);
        wait_version(&web, id, first + 1);
        let reread = get(
            &web,
            &format!("/file?token={}&id={}", web.token(), encode_hex(&id)),
        );
        assert_eq!(reread.body, b"world");
        let reread_session = get(&web, &format!("/session?token={}", web.token()));
        let reread_body: Value = serde_json::from_slice(&reread_session.body).unwrap();
        assert_eq!(reread_body["open-docs"][0]["id"], encode_hex(&id));
        assert_eq!(reread_body["open-docs"][0]["rev"], 2);
        let bumped = web.with_session(|session| session.version());
        assert!(bumped > first);

        web.with_session(|session| session.clear());
        let cleared = web.with_session(|session| session.version());
        assert!(cleared > bumped);
        let empty = get(&web, &format!("/session?token={}", web.token()));
        let empty: Value = serde_json::from_slice(&empty.body).unwrap();
        assert_eq!(empty["open-docs"].as_array().unwrap().len(), 0);
        assert!(empty["active"].is_null());

        let mut ids = Vec::new();
        for index in 0..=MAX_DOCS {
            let path = write_file(
                &dir,
                &format!("f{index}.txt"),
                format!("{index}").as_bytes(),
            );
            ids.push(web.register(&path).unwrap());
        }
        assert!(!web.with_session(|session| session.docs.contains_key(&ids[0])));
        let gone = get(
            &web,
            &format!("/file?token={}&id={}", web.token(), encode_hex(&ids[0])),
        );
        assert_eq!(gone.status, 404);
        assert!(web.with_session(|session| session.docs.contains_key(&ids[MAX_DOCS])));
        assert_eq!(web.with_session(|session| session.order.len()), MAX_DOCS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_file_is_503_and_failures_do_not_serve_foreign_bytes() {
        let web = start().unwrap();
        let (id, generation) = web.with_session(|session| {
            session.begin_doc(
                PathBuf::from("/tmp/pending.txt"),
                FileId { dev: 1, ino: 2 },
                "pending.txt".into(),
            )
        });
        let pending = get(
            &web,
            &format!("/file?token={}&id={}", web.token(), encode_hex(&id)),
        );
        assert_eq!(pending.status, 503);
        assert_eq!(pending.headers["retry-after"], "1");

        web.with_session(|session| {
            finish_read(
                session,
                id,
                generation,
                Some(FileId { dev: 9, ino: 9 }),
                Ok(PreviewText {
                    bytes: b"other".to_vec(),
                    truncated: false,
                }),
            );
        });
        let changed = get(
            &web,
            &format!("/file?token={}&id={}", web.token(), encode_hex(&id)),
        );
        assert_eq!(changed.status, 409);
        assert_ne!(changed.body, b"other");

        let (bad, generation) = web.with_session(|session| {
            session.begin_doc(
                PathBuf::from("/tmp/bad.txt"),
                FileId { dev: 1, ino: 3 },
                "bad.txt".into(),
            )
        });
        web.with_session(|session| {
            finish_read(
                session,
                bad,
                generation,
                Some(FileId { dev: 1, ino: 3 }),
                Err(ReadPreviewError::NotUtf8),
            );
        });
        let rejected = get(
            &web,
            &format!("/file?token={}&id={}", web.token(), encode_hex(&bad)),
        );
        assert_eq!(rejected.status, 415);
    }

    #[test]
    fn post_waits_for_the_body_and_events_follow_the_table() {
        let web = start().unwrap();
        let dir = temp_dir("events");
        let first = web
            .register(&write_file(&dir, "a.rs", b"fn a() {}"))
            .unwrap();
        let second = web
            .register(&write_file(&dir, "b.rs", b"fn b() {}"))
            .unwrap();
        wait_version(&web, first, 1);
        wait_version(&web, second, 1);
        let body = format!(
            "{{\"type\":\"close-tab\",\"id\":\"{}\"}}",
            encode_hex(&second)
        )
        .into_bytes();
        let header = format!(
            "POST /event?token={} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            web.token(),
            web.port(),
            body.len()
        );
        let mut stream = TcpStream::connect(("127.0.0.1", web.port())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(&body[..4]).unwrap();
        thread::sleep(Duration::from_millis(40));
        assert!(
            web.with_session(|session| session.docs.contains_key(&second)),
            "close-tab must wait until Content-Length bytes have arrived"
        );
        stream.write_all(&body[4..]).unwrap();
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf);
        assert!(std::str::from_utf8(&buf)
            .unwrap()
            .starts_with("HTTP/1.1 204"));
        assert!(!web.with_session(|session| session.docs.contains_key(&second)));
        assert_eq!(web.with_session(|session| session.active), Some(first));
        assert!(matches!(
            web.drain_events()[0],
            PreviewEvent::CloseTab { .. }
        ));

        // `first` is already active. Selecting it again must not bump.
        let version = web.with_session(|session| session.version());
        let select = format!(
            "{{\"type\":\"select-tab\",\"id\":\"{}\"}}",
            encode_hex(&first)
        );
        let res = exchange(
            web.port(),
            &format!(
                "POST /event?token={} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{select}",
                web.token(),
                web.port(),
                select.len()
            ),
        );
        assert_eq!(res.status, 204);
        assert_eq!(web.with_session(|session| session.version()), version);
        let _ = exchange(
            web.port(),
            &format!(
                "POST /event?token={} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{select}",
                web.token(),
                web.port(),
                select.len()
            ),
        );
        assert_eq!(web.with_session(|session| session.version()), version);

        let close = b"{\"type\":\"close\"}";
        let res = exchange(
            web.port(),
            &format!(
                "POST /event?token={} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                web.token(),
                web.port(),
                close.len(),
                std::str::from_utf8(close).unwrap()
            ),
        );
        assert_eq!(res.status, 204);
        assert!(web.with_session(|session| session.docs.is_empty()));

        let cross = exchange(
            web.port(),
            &format!(
                "POST /event?token={} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nOrigin: http://evil\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                web.token(),
                web.port()
            ),
        );
        assert_eq!(cross.status, 403);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn connection_limit_and_read_timeout() {
        let web = start_with(Limits {
            max_connections: 2,
            read_timeout: Duration::from_millis(400),
        })
        .unwrap();
        let mut held = Vec::new();
        for _ in 0..2 {
            let mut stream = TcpStream::connect(("127.0.0.1", web.port())).unwrap();
            stream
                .write_all(b"GET /session HTTP/1.1\r\nHost: ")
                .unwrap();
            held.push(stream);
        }
        thread::sleep(Duration::from_millis(50));
        let limited = get(&web, &format!("/session?token={}", web.token()));
        assert_eq!(limited.status, 503);
        drop(held);

        let web = start_with(Limits {
            max_connections: 4,
            read_timeout: Duration::from_millis(200),
        })
        .unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", web.port())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let started = Instant::now();
        let mut buf = [0u8; 8];
        let read = stream.read(&mut buf);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "idle connection was not closed"
        );
        assert!(matches!(read, Ok(0) | Err(_)));
    }

    #[test]
    fn evicting_the_oldest_doc_bumps_and_drops_its_bytes() {
        let web = start().unwrap();
        let (first, generation) = web.with_session(|session| {
            session.begin_doc(
                PathBuf::from("/tmp/old.txt"),
                FileId { dev: 1, ino: 1 },
                "old.txt".into(),
            )
        });
        web.with_session(|session| {
            finish_read(
                session,
                first,
                generation,
                Some(FileId { dev: 1, ino: 1 }),
                Ok(PreviewText {
                    bytes: b"secret".to_vec(),
                    truncated: false,
                }),
            );
        });
        let version = web.with_session(|session| {
            for index in 1..MAX_DOCS {
                session.begin_doc(
                    PathBuf::from(format!("/tmp/f{index}.txt")),
                    FileId {
                        dev: 1,
                        ino: index as u64,
                    },
                    format!("f{index}.txt"),
                );
            }
            session.version()
        });
        web.with_session(|session| {
            session.begin_doc(
                PathBuf::from("/tmp/new.txt"),
                FileId { dev: 1, ino: 99 },
                "new.txt".into(),
            );
        });
        assert!(web.with_session(|session| session.version()) > version);
        assert!(!web.with_session(|session| session.docs.contains_key(&first)));
        let gone = get(
            &web,
            &format!("/file?token={}&id={}", web.token(), encode_hex(&first)),
        );
        assert_eq!(gone.status, 404);
        assert_ne!(gone.body, b"secret");
    }

    #[test]
    fn read_notifies_after_the_session_lock_is_released() {
        let dir = temp_dir("notify");
        let path = write_file(&dir, "n.txt", b"hi");
        let web = start().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        web.set_notifier(Arc::new(move || {
            let _ = tx.send(());
        }));
        web.register(&path).unwrap();
        rx.recv_timeout(Duration::from_secs(2))
            .expect("register did not notify");
        let view = web.session_view();
        assert_eq!(view.docs.len(), 1);
        assert_eq!(view.active, Some(view.docs[0].id));
        assert_eq!(view.docs[0].name, "n.txt");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
