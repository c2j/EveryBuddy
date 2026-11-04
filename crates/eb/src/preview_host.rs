//! The one preview host the shell talks to.
//!
//! macOS embeds a `WKWebView` (`preview_panel`). Windows embeds WebView2
//! (`preview_web`). Linux embeds nothing unless `EB_PREVIEW=webview`, in
//! which case `preview_linux` adopts a helper plug. Callers present, move,
//! and wake one host; they do not name the platform modules.

use crate::model::{App, PreviewMode, Route};
use crate::preview_web_server::{FileId, PreviewText, ReadPreviewError};

/// Show the web view over `anchor` (the artifact body).
pub fn present(anchor: &gtk::Widget) {
    crate::preview_panel::present(anchor);
    crate::preview_web::present();
    #[cfg(target_os = "linux")]
    crate::preview_linux::present();
}

pub fn dismiss() {
    crate::preview_panel::dismiss();
    crate::preview_web::dismiss();
    #[cfg(target_os = "linux")]
    crate::preview_linux::dismiss();
}

pub fn reposition() {
    crate::preview_panel::reposition();
    crate::preview_web::reposition();
}

/// Finish a present that returned before the anchor had a size.
pub fn retry_present(anchor: &gtk::Widget) {
    crate::preview_panel::retry_present(anchor);
    #[cfg(target_os = "linux")]
    crate::preview_linux::retry_present();
}

pub fn wake() {
    crate::preview_panel::wake();
    crate::preview_web::wake();
    #[cfg(target_os = "linux")]
    crate::preview_linux::wake();
}

/// Install the macOS click filter on the main window. A no-op elsewhere.
pub fn follow(window: &gtk::Window) {
    crate::preview_panel::follow(window);
}

/// Hide the macOS view while the main window is minimized.
pub fn set_suspended(suspended: bool) {
    crate::preview_panel::set_suspended(suspended);
}

/// Keep the Windows drawing area. A no-op off Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn attach(anchor: gtk::DrawingArea) {
    crate::preview_web::attach(anchor);
}

/// Keep the Linux socket. It exists only when `EB_PREVIEW=webview`.
#[cfg(target_os = "linux")]
pub fn attach_socket(socket: gtk::Socket) {
    crate::preview_linux::attach(socket);
}

/// Runs when an embed failure is noticed after `refresh` has returned.
pub fn set_late_fallback(handler: std::rc::Rc<dyn Fn()>) {
    #[cfg(target_os = "linux")]
    crate::preview_linux::set_late_fallback(std::rc::Rc::clone(&handler));
    crate::preview_web::set_late_fallback(handler);
}

/// The reason from the latest embed failure, if `refresh` has not taken it yet.
pub fn take_fallback() -> Option<String> {
    if let Some(reason) = crate::preview_web::take_fallback() {
        return Some(reason);
    }
    #[cfg(target_os = "linux")]
    {
        return crate::preview_linux::take_fallback();
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// Leave `WebView` for the rest of the process and show the text view.
///
/// A caller that already filled `tool_preview` (a register that never reached
/// the session) keeps that text. Otherwise the active session document is
/// copied. Does not ask for a repaint; the caller does.
pub(crate) fn fall_back_to_native(app: &mut App, reason: &str) {
    eprintln!("[evobuddy] {reason}");
    app.preview_mode = PreviewMode::StickyNative;
    if app.tool_preview.is_some() {
        return;
    }
    let Some(task) = (match &app.route {
        Route::Task(index) => Some(*index),
        _ => None,
    }) else {
        return;
    };
    let Some(doc) = crate::preview_web_server::session_view_shared()
        .as_ref()
        .and_then(|view| view.active_doc().cloned())
    else {
        return;
    };
    let cached = crate::preview_web_server::ready_text_shared(doc.id);
    // One open: the identity and the bytes come from the same handle, so the
    // 409 check uses the same `FileId` the register path recorded.
    let opened = if cached.is_none() {
        std::fs::File::open(std::path::Path::new(&doc.path)).ok()
    } else {
        None
    };
    let observed = opened
        .as_ref()
        .and_then(|file| crate::preview_web_server::file_identity(file).ok());
    let read = opened.map(|file| {
        crate::preview_web_server::read_opened(file, crate::preview_web_server::PREVIEW_CAP)
    });
    app.tool_preview = Some(crate::model::Artifact {
        name: doc.name,
        code: sticky_fallback_text(cached, read, doc.file_id, observed),
    });
    app.tool_preview_task = Some(task);
}

pub(crate) fn native_preview_code(text: &str, truncated: bool) -> String {
    let mut code = text.to_string();
    if truncated {
        code.push_str("\n… (truncated at 5 MiB)");
    }
    code
}

/// Cached bytes win. A cache miss is read on this thread and checked against
/// the id captured at register time. A mismatch does not show the other file.
pub(crate) fn sticky_fallback_text(
    cached: Option<(String, bool)>,
    read: Option<Result<PreviewText, ReadPreviewError>>,
    registered: FileId,
    observed: Option<FileId>,
) -> String {
    if let Some((text, truncated)) = cached {
        return native_preview_code(&text, truncated);
    }
    match read {
        Some(Ok(text)) if observed == Some(registered) => {
            native_preview_code(text.as_str(), text.truncated)
        }
        Some(Ok(_)) => "The file changed on disk and was not shown.".to_string(),
        Some(Err(ReadPreviewError::NotUtf8)) => "This file is not UTF-8 text.".to_string(),
        _ => "This file is no longer available.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Artifact;

    #[test]
    fn a_prepared_text_view_is_kept_when_the_mode_goes_sticky() {
        let mut app = App::new();
        app.preview_mode = PreviewMode::WebView;
        app.tool_preview = Some(Artifact {
            name: "kept.txt".into(),
            code: "body".into(),
        });
        fall_back_to_native(&mut app, "preview server did not register the file");
        assert_eq!(app.preview_mode, PreviewMode::StickyNative);
        assert_eq!(app.tool_preview.unwrap().name, "kept.txt");
    }
}
