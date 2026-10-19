//! Validation-only native WKWebView window for the archived dsh ui-theme bundle.
//!
//! The first attempt put the WKWebView inside a GTK window's NSView. That
//! rendered, but mouse events never reached WebKit's content view. This version
//! uses a native NSWindow whose contentView *is* the WKWebView, so the proof
//! exercises the same input path a product panel would need.
#![cfg(target_os = "macos")]

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSFloatingWindowLevel, NSView, NSWindow,
    NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

thread_local! {
    static PANEL: std::cell::RefCell<Option<Retained<NSWindow>>> = std::cell::RefCell::new(None);
    static PANEL_ANCHOR: std::cell::RefCell<Option<gtk::Widget>> = std::cell::RefCell::new(None);
    static EMBEDDED: std::cell::RefCell<Option<Retained<WKWebView>>> = std::cell::RefCell::new(None);
    static EMBED_ANCHOR: std::cell::RefCell<Option<gtk::Widget>> = std::cell::RefCell::new(None);
    static MONITOR: std::cell::RefCell<Option<Retained<objc2::runtime::AnyObject>>> = std::cell::RefCell::new(None);
}

#[link(name = "gdk-3.0")]
extern "C" {
    fn gdk_quartz_window_get_nsview(window: *mut std::ffi::c_void) -> *mut NSView;
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
    window.setTitle(&NSString::from_str("dsh feedback"));
    window.setTitleVisibility(objc2_app_kit::NSWindowTitleVisibility::Hidden);
    window.setTitlebarAppearsTransparent(true);
    window.setHasShadow(false);

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
    if std::env::var_os("EB_PANEL_DEBUG").is_some() {
        eprintln!(
            "panel: toplevel={:?} anchor={:?} at=({}, {}) alloc={}x{}",
            toplevel.size(),
            anchor.allocation(),
            wx,
            wy,
            allocation.width(),
            allocation.height()
        );
    }
    let window_size = toplevel.size();
    let width = allocation.width() as f64;
    let height = (allocation.height() as f64).min(window_size.1 as f64 - ay as f64 - 8.0);
    let x = wx as f64 + ax as f64;
    let y = screen_height - (wy as f64 + ay as f64) - height;
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
    let window: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView,
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
    window.setLevel(NSFloatingWindowLevel);
    if let Some(gdk_window) = toplevel.window() {
        let native: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
        let nsview = unsafe { gdk_quartz_window_get_nsview(native as *mut std::ffi::c_void) };
        if !nsview.is_null() {
            let parent = unsafe { &*nsview };
            if let Some(parent_window) = parent.window() {
                unsafe { parent_window.addChildWindow_ordered(&window, NSWindowOrderingMode::Above) };
            }
        }
    }
    window.orderFrontRegardless();
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
    anchor.set_opacity(0.0);
    PANEL_ANCHOR.with(|slot| *slot.borrow_mut() = Some(anchor.clone().upcast()));
    PANEL.with(|panel| *panel.borrow_mut() = Some(window));
}

pub fn close_panel() {
    PANEL_ANCHOR.with(|slot| {
        if let Some(anchor) = slot.borrow_mut().take() {
            anchor.set_opacity(1.0);
        }
    });
    PANEL.with(|panel| {
        if let Some(window) = panel.borrow_mut().take() {
            window.orderOut(None);
        }
    });
}

/// Embed the component host as a real NSView subview of the GTK window.
///
/// GTK installs its own macOS event routing, so a bare subview paints but does
/// not receive mouse events. A local NSEvent monitor forwards events whose
/// location falls inside the webview and consumes them before GTK sees them.
pub fn open_embedded(index: &str, anchor: &gtk::Widget) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    close_embedded();
    let Some(toplevel) = anchor.toplevel().and_downcast::<gtk::Window>() else { return };
    let Some(gdk_window) = toplevel.window() else { return };
    let native: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
    let nsview = unsafe { gdk_quartz_window_get_nsview(native as *mut std::ffi::c_void) };
    if nsview.is_null() { return }
    let parent = unsafe { &*nsview };
    let (ax, ay) = anchor
        .translate_coordinates(&toplevel, 0, 0)
        .unwrap_or((0, 0));
    let allocation = anchor.allocation();
    let parent_height = parent.bounds().size.height;
    let width = allocation.width() as f64;
    let height = (allocation.height() as f64).min(parent_height - ay as f64 - 8.0);
    anchor.set_opacity(0.0);
    EMBED_ANCHOR.with(|slot| *slot.borrow_mut() = Some(anchor.clone()));
    let frame = NSRect::new(
        NSPoint::new(ax as f64, parent_height - ay as f64 - height),
        NSSize::new(width, height),
    );
    if std::env::var_os("EB_PANEL_DEBUG").is_some() {
        eprintln!("embed: toplevel={:?} anchor=({}, {}) alloc={}x{} frame={:?}", toplevel.size(), ax, ay, allocation.width(), allocation.height(), frame);
    }
    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    let webview: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config)
    };
    webview.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
    parent.addSubview(&webview);
    let url = NSURL::URLWithString(&NSString::from_str(index)).expect("valid embedded URL");
    let request = NSURLRequest::requestWithURL(&url);
    unsafe { webview.loadRequest(&request) };
    toplevel.present();
    webview.window().unwrap().makeFirstResponder(Some(&*webview));

    let shot = webview.clone();
    let mask = NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseUp
        | NSEventMask::LeftMouseDragged
        | NSEventMask::MouseMoved
        | NSEventMask::ScrollWheel
        | NSEventMask::KeyDown;
    use block2::RcBlock;
    use std::ptr::NonNull;
    let shot_for_monitor = shot.clone();
    let block = RcBlock::new(move |event: NonNull<NSEvent>| {
        let event = unsafe { event.as_ref() };
        let Some(event_window) = event.window(mtm) else {
            return NonNull::from(event).as_ptr();
        };
        let Some(web_window) = shot_for_monitor.window() else {
            return NonNull::from(event).as_ptr();
        };
        if &*web_window as *const _ != &*event_window as *const _ {
            return NonNull::from(event).as_ptr();
        }
        let point = event.locationInWindow();
        let frame = shot_for_monitor.frame();
        if point.x < frame.origin.x
            || point.y < frame.origin.y
            || point.x > frame.origin.x + frame.size.width
            || point.y > frame.origin.y + frame.size.height
        {
            return NonNull::from(event).as_ptr();
        }
        match event.r#type() {
            NSEventType::LeftMouseDown => shot_for_monitor.mouseDown(event),
            NSEventType::LeftMouseUp => shot_for_monitor.mouseUp(event),
            NSEventType::LeftMouseDragged => shot_for_monitor.mouseDragged(event),
            NSEventType::MouseMoved => shot_for_monitor.mouseMoved(event),
            NSEventType::ScrollWheel => shot_for_monitor.scrollWheel(event),
            NSEventType::KeyDown => shot_for_monitor.keyDown(event),
            _ => return NonNull::from(event).as_ptr(),
        }
        std::ptr::null_mut()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block)
    };
    EMBEDDED.with(|slot| *slot.borrow_mut() = Some(webview));
    MONITOR.with(|slot| *slot.borrow_mut() = monitor);

    if std::env::var_os("EB_PANEL_SHOT").is_some() {
        let shot = EMBEDDED.with(|slot| slot.borrow().clone()).unwrap();
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
                    rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
                }) else { return };
                let path = std::env::var("EB_PANEL_SHOT").unwrap_or_default();
                if png.writeToFile_atomically(&NSString::from_str(&path), true) {
                    eprintln!("panel: wrote {path}");
                }
            });
            unsafe { shot.takeSnapshotWithConfiguration_completionHandler(None, &block) };
        });
    }
}

pub fn close_embedded() {
    EMBED_ANCHOR.with(|slot| {
        if let Some(anchor) = slot.borrow_mut().take() {
            anchor.set_opacity(1.0);
        }
    });
    MONITOR.with(|slot| {
        if let Some(token) = slot.borrow_mut().take() {
            unsafe { NSEvent::removeMonitor(&token) };
        }
    });
    EMBEDDED.with(|slot| {
        if let Some(webview) = slot.borrow_mut().take() {
            webview.removeFromSuperview();
        }
    });
}
