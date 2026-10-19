//! Validation-only native WKWebView window for the archived dsh ui-theme bundle.
//!
//! The first attempt put the WKWebView inside a GTK window's NSView. That
//! rendered, but mouse events never reached WebKit's content view. This version
//! uses a native NSWindow whose contentView *is* the WKWebView, so the proof
//! exercises the same input path a product panel would need.
#![cfg(target_os = "macos")]

use gtk::prelude::*;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

thread_local! {
    static PANEL: std::cell::RefCell<Option<Retained<NSWindow>>> = std::cell::RefCell::new(None);
}

pub fn open(index: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let frame = NSRect::new(NSPoint::new(80.0, 80.0), NSSize::new(760.0, 560.0));
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable;
    let window: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str("dsh ui-theme — component host"));
    unsafe { window.setReleasedWhenClosed(false) };

    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let webview: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config)
    };
    webview.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    window.setContentView(Some(&webview));
    window.makeKeyAndOrderFront(None);
    window.center();
    window.makeFirstResponder(Some(&*webview));
    webview.setFrame(window.contentView().map(|view| view.bounds()).unwrap_or(frame));

    if index.starts_with("http://") || index.starts_with("https://") {
        let url = NSURL::URLWithString(&NSString::from_str(index)).expect("valid preview URL");
        let request = NSURLRequest::requestWithURL(&url);
        unsafe { webview.loadRequest(&request) };
    } else {
        let url = NSURL::fileURLWithPath(&NSString::from_str(index));
        let root = NSURL::fileURLWithPath(&NSString::from_str(
            std::path::Path::new(index)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_str()
                .unwrap_or("."),
        ));
        unsafe {
            webview.loadFileURL_allowingReadAccessToURL(&url, &root);
        }
    }

    if let Ok(path) = std::env::var("EB_THEME_SHOT") {
        let shot = webview.clone();
        let path = path.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            use block2::RcBlock;
            use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
            use objc2_foundation::{NSDictionary, NSError};
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if !error.is_null() || image.is_null() {
                    eprintln!("theme: snapshot failed");
                    return;
                }
                let image = unsafe { &*image };
                let Some(tiff) = image.TIFFRepresentation() else { return };
                let Some(rep) = NSBitmapImageRep::imageRepWithData(&tiff) else { return };
                let Some(png) = (unsafe {
                    rep.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }) else {
                    return;
                };
                if png.writeToFile_atomically(&NSString::from_str(&path), true) {
                    eprintln!("theme: wrote {path}");
                }
            });
            unsafe {
                shot.takeSnapshotWithConfiguration_completionHandler(None, &block);
            }
        });
    }

    // Keep the native window alive for the process lifetime. It is an NSWindow,
    // not a GTK window, so the GTK application loop does not own it.
    std::mem::forget(window);
}

/// Open the component host as a borderless native panel anchored over a GTK
/// widget (the artifact panel). This is the right-sidebar integration path that
/// keeps WebKit input working.
pub fn open_panel(index: &str, anchor: &gtk::Widget) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    close_panel();
    let Some(toplevel) = anchor.toplevel().and_downcast::<gtk::Window>() else { return };
    let (ax, ay) = anchor
        .translate_coordinates(&toplevel, 0, 0)
        .unwrap_or((0, 0));
    let allocation = anchor.allocation();
    let (wx, wy) = toplevel.position();
    let screen_height = objc2_app_kit::NSScreen::mainScreen(mtm)
        .map(|screen| screen.frame().size.height)
        .unwrap_or(900.0);
    let (window_width, window_height) = toplevel.size();
    let width = (allocation.width() as f64).clamp(360.0, 520.0);
    let height = (window_height as f64 - 96.0)
        .min(allocation.height() as f64)
        .max(420.0);
    let x = wx as f64 + window_width as f64 - width;
    let y = screen_height - (wy as f64 + 48.0) - height;
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
    let window: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let webview: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config)
    };
    webview.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    window.setContentView(Some(&webview));
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&*webview));
    if index.starts_with("http://") || index.starts_with("https://") {
        let url = NSURL::URLWithString(&NSString::from_str(index)).expect("valid panel URL");
        let request = NSURLRequest::requestWithURL(&url);
        unsafe { webview.loadRequest(&request) };
    }
    if std::env::var_os("EB_PANEL_SHOT").is_some() {
        let shot = webview.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            use block2::RcBlock;
            use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
            use objc2_foundation::{NSDictionary, NSError};
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if !error.is_null() || image.is_null() { return; }
                let image = unsafe { &*image };
                let Some(tiff) = image.TIFFRepresentation() else { return };
                let Some(rep) = NSBitmapImageRep::imageRepWithData(&tiff) else { return };
                let Some(png) = (unsafe {
                    rep.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }) else { return };
                let path = std::env::var("EB_PANEL_SHOT").unwrap_or_default();
                if png.writeToFile_atomically(&NSString::from_str(&path), true) {
                    eprintln!("panel: wrote {path}");
                }
            });
            unsafe { shot.takeSnapshotWithConfiguration_completionHandler(None, &block) };
        });
    }
    PANEL.with(|panel| *panel.borrow_mut() = Some(window));
}

pub fn close_panel() {
    PANEL.with(|panel| {
        if let Some(window) = panel.borrow_mut().take() {
            window.orderOut(None);
        }
    });
}
