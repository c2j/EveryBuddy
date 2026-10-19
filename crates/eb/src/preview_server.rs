//! Tiny localhost static server for the WebKit component-host assets.
//!
//! WKWebView refuses to reliably call the loopback bridge from a `file://`
//! document, so previews and the right panel load from
//! `http://127.0.0.1:39473/`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Once;

const INDEX: &str = include_str!("../../../experiments/ui-feedback-webview/index.html");
const INDEX_LIKE: &str = include_str!("../../../experiments/ui-feedback-webview/index-like.html");
const INDEX_SUBMIT: &str = include_str!("../../../experiments/ui-feedback-webview/index-submit.html");
const HOST_JS: &[u8] = include_bytes!("../../../experiments/ui-feedback-webview/host.js");
const FEEDBACK_JS: &[u8] =
    include_bytes!("../../../experiments/ui-feedback-webview/feedback-client.js");
const THEME_INDEX: &str = include_str!("../../../experiments/ui-theme-webview/index.html");
const THEME_JS: &[u8] = include_bytes!("../../../experiments/ui-theme-webview/host.js");
const THEME_CLIENT: &[u8] =
    include_bytes!("../../../experiments/ui-theme-webview/theme-client.js");

static START: Once = Once::new();

pub fn ensure() {
    START.call_once(|| {
        std::thread::spawn(|| {
            let Ok(listener) = TcpListener::bind("127.0.0.1:39473") else { return };
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = [0u8; 2048];
                let Ok(read) = stream.read(&mut request) else { continue };
                let request = String::from_utf8_lossy(&request[..read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (body, content_type): (&[u8], &str) = match path {
                    "/" | "/index.html" => (INDEX.as_bytes(), "text/html; charset=utf-8"),
                    "/index-like.html" => (INDEX_LIKE.as_bytes(), "text/html; charset=utf-8"),
                    "/index-submit.html" => (INDEX_SUBMIT.as_bytes(), "text/html; charset=utf-8"),
                    "/host.js" => (HOST_JS, "text/javascript; charset=utf-8"),
                    "/feedback-client.js" => (FEEDBACK_JS, "text/javascript; charset=utf-8"),
                    "/theme/index.html" => (THEME_INDEX.as_bytes(), "text/html; charset=utf-8"),
                    "/theme/host.js" => (THEME_JS, "text/javascript; charset=utf-8"),
                    "/theme/theme-client.js" => (THEME_CLIENT, "text/javascript; charset=utf-8"),
                    _ => (b"not found", "text/plain; charset=utf-8"),
                };
                let status = if path.starts_with("/host.js")
                    || path.starts_with("/feedback-client.js")
                    || path.starts_with("/theme/")
                    || path.starts_with("/index")
                    || path == "/"
                {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            }
        });
    });
}

pub fn url() -> &'static str {
    "http://127.0.0.1:39473/index.html"
}

pub fn theme_url() -> &'static str {
    "http://127.0.0.1:39473/theme/index.html"
}
