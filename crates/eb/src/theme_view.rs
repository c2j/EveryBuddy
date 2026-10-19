//! Validation-only WKWebView host for the archived dsh ui-theme bundle.
#![cfg(target_os = "macos")]

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
use objc2_foundation::{NSString, NSURL};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

#[link(name = "gdk-3.0")]
extern "C" {
    fn gdk_quartz_window_get_nsview(window: *mut std::ffi::c_void) -> *mut NSView;
}

pub fn open(index: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("dsh ui-theme — component host");
    window.set_default_size(760, 560);
    let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&host);
    window.show_all();
    window.present();
    host.realize();
    let Some(gdk_window) = host.window() else { return };
    let native: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
    let nsview = unsafe { gdk_quartz_window_get_nsview(native as *mut std::ffi::c_void) };
    if nsview.is_null() { return }
    let parent: &NSView = unsafe { &*nsview };

    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let webview: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), parent.bounds(), &config)
    };
    webview.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    parent.addSubview(&webview);
    if let Some(ns_window) = parent.window() {
        ns_window.makeKeyAndOrderFront(None);
        unsafe {
            ns_window.makeFirstResponder(Some(&*webview));
        }
    }

    let url = NSURL::fileURLWithPath(&NSString::from_str(index));
    let root = NSURL::fileURLWithPath(&NSString::from_str(
        std::path::Path::new(index).parent().unwrap_or(std::path::Path::new(".")).to_str().unwrap_or("."),
    ));
    unsafe { webview.loadFileURL_allowingReadAccessToURL(&url, &root); }

    if let Ok(path) = std::env::var("EB_THEME_SHOT") {
        let shot = webview.clone();
        let path = path.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            use block2::RcBlock;
            use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
            use objc2_foundation::{NSDictionary, NSError};
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if !error.is_null() || image.is_null() { eprintln!("theme: snapshot failed"); return; }
                let image = unsafe { &*image };
                let Some(tiff) = image.TIFFRepresentation() else { return };
                let Some(rep) = NSBitmapImageRep::imageRepWithData(&tiff) else { return };
                let Some(png) = (unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }) else { return };
                if let Ok(mut file) = std::fs::File::create(&path) {
                    let _ = png.writeToFile_atomically(&NSString::from_str(&path), true);
                    let _ = &mut file;
                    eprintln!("theme: wrote {path}");
                }
            });
            unsafe { shot.takeSnapshotWithConfiguration_completionHandler(None, &block) };
        });
    }
}
