//! A real DSH Web view for macOS.
//!
//! The dsh client plugins are React/Cordis modules that need a browser DOM.
//! Rather than porting them one by one, this opens the actual `dsh web` URL in
//! a WKWebView. It is the first slice of the WebView integration: the URL and
//! the sidecar lifecycle are supplied by the caller (`EB_DSH_WEB_URL` for the
//! harness), while the GTK app remains the window shell.
#![cfg(target_os = "macos")]

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use block2::RcBlock;
use objc2_app_kit::{NSAutoresizingMaskOptions, NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSView};
use objc2_foundation::{NSDictionary, NSError, NSRect, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

#[link(name = "gdk-3.0")]
extern "C" {
    fn gdk_quartz_window_get_nsview(window: *mut std::ffi::c_void) -> *mut NSView;
}

/// Open the dsh Web UI in a WKWebView-backed GTK window.
pub fn open(url: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("dsh-web: not on the main thread");
        return;
    };
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("DSH Web — plugins");
    window.set_default_size(1280, 840);
    let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&host);
    window.show_all();
    host.realize();

    let Some(gdk_window) = host.window() else {
        eprintln!("dsh-web: host has no GDK window");
        return;
    };
    let native: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
    let native = native as *mut std::ffi::c_void;
    let nsview = unsafe { gdk_quartz_window_get_nsview(native) };
    if nsview.is_null() {
        eprintln!("dsh-web: GDK window has no NSView");
        return;
    }
    let parent: &NSView = unsafe { &*nsview };
    let frame: NSRect = parent.bounds();

    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let webview: Retained<WKWebView> =
        unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config) };
    webview.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    parent.addSubview(&webview);

    let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(url)) else {
        eprintln!("dsh-web: invalid URL {url}");
        return;
    };
    let request = NSURLRequest::requestWithURL(&nsurl);
    unsafe {
        webview.loadRequest(&request);
    }

    let probe = webview.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(5), move || {
        unsafe {
            eprintln!(
                "dsh-web: title={:?} loading={}",
                probe.title().map(|value| value.to_string()),
                probe.isLoading()
            );
        }
    });

    if let Ok(script) = std::env::var("EB_DSH_JS") {
        let page = webview.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
            unsafe {
                page.evaluateJavaScript_completionHandler(&NSString::from_str(&script), None);
            }
        });
    }

    if let Ok(path) = std::env::var("EB_DSH_SNAPSHOT") {
        let shot = webview.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(7), move || {
            let path = path.clone();
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if !error.is_null() {
                    eprintln!("dsh-web: snapshot failed");
                    return;
                }
                if image.is_null() {
                    eprintln!("dsh-web: snapshot returned no image");
                    return;
                }
                let image = unsafe { &*image };
                let Some(tiff) = image.TIFFRepresentation() else {
                    eprintln!("dsh-web: no TIFF representation");
                    return;
                };
                let Some(rep) = NSBitmapImageRep::imageRepWithData(&tiff) else {
                    eprintln!("dsh-web: no bitmap representation");
                    return;
                };
                let Some(png) = (unsafe {
                    rep.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                }) else {
                    eprintln!("dsh-web: PNG encoding failed");
                    return;
                };
                if png.writeToFile_atomically(&NSString::from_str(&path), true) {
                    eprintln!("dsh-web: snapshot wrote {path}");
                } else {
                    eprintln!("dsh-web: snapshot write failed");
                }
            });
            unsafe {
                shot.takeSnapshotWithConfiguration_completionHandler(None, &block);
            }
        });
    }
}
