//! macOS preview host. One `WKWebView` is a subview of the GTK window's
//! content view, aligned to `detail.body`.
//!
//! The view moves with the window, so a drag has no second frame to catch up.
//! Quartz points are not multiplied by the HiDPI scale. GDK dequeues mouse
//! events before AppKit hit-tests, so a window filter hands events inside the
//! view back to AppKit. It does not use the monitor or event tap in
//! `theme_view.rs`.

/// Rectangle of the anchor inside an unflipped content view, in points.
/// The origin is the bottom-left of that view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `ay` is the distance from the content top, in points.
pub fn view_frame(parent_height: f64, ax: i32, ay: i32, width: i32, height: i32) -> ViewFrame {
    ViewFrame {
        x: f64::from(ax),
        y: parent_height - f64::from(ay) - f64::from(height),
        width: f64::from(width),
        height: f64::from(height),
    }
}

pub fn present(anchor: &gtk::Widget) {
    #[cfg(target_os = "macos")]
    macos::present(anchor);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = anchor;
    }
}

pub fn dismiss() {
    #[cfg(target_os = "macos")]
    macos::dismiss();
}

pub fn reposition() {
    #[cfg(target_os = "macos")]
    macos::reposition();
}

/// Install the event filter on the main window. The web view is a subview,
/// so a drag does not need a follower. Settings windows are not observed.
pub fn follow(window: &gtk::Window) {
    #[cfg(target_os = "macos")]
    macos::follow(window);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
    }
}

/// Unchanged size, within half a point. Callers still compare the origin.
fn origin_move(current_width: f64, current_height: f64, width: f64, height: f64) -> bool {
    (current_width - width).abs() < 0.5 && (current_height - height).abs() < 0.5
}

/// Ask the loaded page to pull `/session` now. The script carries no document
/// bytes. A repeat of the same session version does nothing.
pub fn wake() {
    #[cfg(target_os = "macos")]
    macos::wake();
}

/// Hide the view while the main window is minimized, and show it again
/// without taking key when the window returns.
pub fn set_suspended(suspended: bool) {
    #[cfg(target_os = "macos")]
    macos::set_suspended(suspended);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = suspended;
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::{Cell, RefCell};

    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSAutoresizingMaskOptions, NSEvent, NSEventType, NSView, NSWindow};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURL};
    use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

    use super::view_frame;

    #[link(name = "gdk-3.0")]
    extern "C" {
        fn gdk_quartz_window_get_nsview(window: *mut std::ffi::c_void) -> *mut NSView;
        fn gdk_quartz_window_get_nswindow(window: *mut std::ffi::c_void) -> *mut NSWindow;
        fn gdk_window_add_filter(
            window: *mut gtk::gdk::ffi::GdkWindow,
            function: unsafe extern "C" fn(
                *mut std::ffi::c_void,
                *mut std::ffi::c_void,
                *mut std::ffi::c_void,
            ) -> i32,
            data: *mut std::ffi::c_void,
        );
    }

    const FILTER_CONTINUE: i32 = 0;
    const FILTER_REMOVE: i32 = 2;

    struct Host {
        web: Retained<WKWebView>,
        loaded: bool,
        visible: bool,
        suspended: bool,
        /// Session version last handed to the page. `None` until the first poke.
        woken: Option<u64>,
        anchor: gtk::Widget,
    }

    thread_local! {
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
        static FILTER_INSTALLED: Cell<bool> = const { Cell::new(false) };
    }

    pub fn follow(window: &gtk::Window) {
        ensure_filter(window.upcast_ref());
    }

    fn ensure_filter(anchor: &gtk::Widget) {
        if FILTER_INSTALLED.with(|cell| cell.get()) {
            return;
        }
        let Some(toplevel) = anchor.toplevel().and_downcast::<gtk::Window>() else {
            return;
        };
        let Some(gdk_window) = toplevel.window() else {
            return;
        };
        let native: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
        unsafe {
            gdk_window_add_filter(native, preview_event_filter, std::ptr::null_mut());
        }
        FILTER_INSTALLED.with(|cell| cell.set(true));
    }

    /// Quartz takes the event out of the queue and, when it can translate it,
    /// never calls `sendEvent`. Returning remove makes that translation fail
    /// so AppKit hit-tests the web view. A tap is not involved.
    unsafe extern "C" fn preview_event_filter(
        xevent: *mut std::ffi::c_void,
        _event: *mut std::ffi::c_void,
        _data: *mut std::ffi::c_void,
    ) -> i32 {
        if xevent.is_null() {
            return FILTER_CONTINUE;
        }
        let event = unsafe { &*xevent.cast::<NSEvent>() };
        if claims_event(event) {
            FILTER_REMOVE
        } else {
            FILTER_CONTINUE
        }
    }

    fn claims_event(event: &NSEvent) -> bool {
        HOST.with(|slot| {
            let Ok(slot) = slot.try_borrow() else {
                return false;
            };
            let Some(host) = slot.as_ref() else {
                return false;
            };
            if !host.visible || host.suspended || host.web.isHidden() {
                return false;
            }
            let kind = event.r#type();
            if is_key(kind) {
                return focus_is_in(&host.web);
            }
            if is_pointer(kind) {
                return point_in_web(&host.web, event);
            }
            false
        })
    }

    fn is_key(kind: NSEventType) -> bool {
        matches!(
            kind,
            NSEventType::KeyDown | NSEventType::KeyUp | NSEventType::FlagsChanged
        )
    }

    fn is_pointer(kind: NSEventType) -> bool {
        matches!(
            kind,
            NSEventType::LeftMouseDown
                | NSEventType::LeftMouseUp
                | NSEventType::LeftMouseDragged
                | NSEventType::RightMouseDown
                | NSEventType::RightMouseUp
                | NSEventType::RightMouseDragged
                | NSEventType::OtherMouseDown
                | NSEventType::OtherMouseUp
                | NSEventType::OtherMouseDragged
                | NSEventType::MouseMoved
                | NSEventType::ScrollWheel
                | NSEventType::Magnify
                | NSEventType::Swipe
                | NSEventType::Rotate
                | NSEventType::SmartMagnify
                | NSEventType::Pressure
        )
    }

    fn focus_is_in(web: &WKWebView) -> bool {
        let Some(window) = web.window() else {
            return false;
        };
        let Some(responder) = window.firstResponder() else {
            return false;
        };
        let Ok(mut view) = responder.downcast::<NSView>() else {
            return false;
        };
        let target: &NSView = web;
        loop {
            if std::ptr::eq(view.as_ref(), target) {
                return true;
            }
            let Some(parent) = (unsafe { view.superview() }) else {
                return false;
            };
            view = parent;
        }
    }

    fn point_in_web(web: &WKWebView, event: &NSEvent) -> bool {
        let Some(parent) = (unsafe { web.superview() }) else {
            return false;
        };
        let frame = web.frame();
        if frame.size.width <= 0.0 || frame.size.height <= 0.0 {
            return false;
        }
        let point = parent.convertPoint_fromView(event.locationInWindow(), None);
        point.x >= frame.origin.x
            && point.y >= frame.origin.y
            && point.x <= frame.origin.x + frame.size.width
            && point.y <= frame.origin.y + frame.size.height
    }

    pub fn present(anchor: &gtk::Widget) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(url) = crate::preview_web_server::shared_preview_url() else {
            return;
        };
        ensure_filter(anchor);
        let Some(parent) = content_view(anchor) else {
            return;
        };
        let Some(rect) = frame_for(anchor, &parent) else {
            return;
        };
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(create(mtm, rect, anchor));
            }
            let Some(host) = slot.as_mut() else {
                return;
            };
            host.anchor = anchor.clone();
            show(&host.web, &parent, rect, host.suspended);
            if !host.loaded {
                host.loaded = load(&host.web, &url);
            }
            host.visible = true;
        });
    }

    pub fn dismiss() {
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(host) = slot.as_mut() else {
                return;
            };
            if !host.visible {
                return;
            }
            let suspended = host.suspended;
            host.visible = false;
            hide(&host.web);
            // Returning key while the GTK window is minimized would unminimize it.
            if !suspended {
                return_key(&host.anchor);
            }
        });
    }

    pub fn wake() {
        let Some(version) =
            crate::preview_web_server::session_view_shared().map(|view| view.version)
        else {
            return;
        };
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(host) = slot.as_mut() else {
                return;
            };
            if !host.loaded || !host.visible || host.suspended || host.woken == Some(version) {
                return;
            }
            host.woken = Some(version);
            poke(&host.web);
        });
    }

    pub fn set_suspended(suspended: bool) {
        let version = crate::preview_web_server::session_view_shared().map(|view| view.version);
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(host) = slot.as_mut() else {
                return;
            };
            if host.suspended == suspended {
                return;
            }
            host.suspended = suspended;
            if !host.visible {
                return;
            }
            if suspended {
                hide(&host.web);
                return;
            }
            if let Some(parent) = content_view(&host.anchor) {
                if let Some(rect) = frame_for(&host.anchor, &parent) {
                    show(&host.web, &parent, rect, false);
                }
            }
            if host.loaded {
                host.woken = version;
                poke(&host.web);
            }
        });
    }

    pub fn reposition() {
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(host) = slot.as_mut() else {
                return;
            };
            if !host.visible || host.suspended {
                return;
            }
            let Some(parent) = content_view(&host.anchor) else {
                return;
            };
            let Some(rect) = frame_for(&host.anchor, &parent) else {
                return;
            };
            show(&host.web, &parent, rect, false);
        });
    }

    fn create(mtm: MainThreadMarker, rect: NSRect, anchor: &gtk::Widget) -> Host {
        let web = unsafe {
            let config = WKWebViewConfiguration::new(mtm);
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), rect, &config)
        };
        // The anchor moves on its own. A sizable mask would stretch the view
        // from the corner before the next allocation arrives.
        web.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);
        Host {
            web,
            loaded: false,
            visible: false,
            suspended: false,
            woken: None,
            anchor: anchor.clone(),
        }
    }

    fn content_view(anchor: &gtk::Widget) -> Option<Retained<NSView>> {
        let toplevel = anchor.toplevel().and_downcast::<gtk::Window>()?;
        let gdk_window = toplevel.window()?;
        let native: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
        let ptr = unsafe { gdk_quartz_window_get_nsview(native as *mut std::ffi::c_void) };
        unsafe { Retained::retain(ptr) }
    }

    fn frame_for(anchor: &gtk::Widget, parent: &NSView) -> Option<NSRect> {
        let toplevel = anchor.toplevel().and_downcast::<gtk::Window>()?;
        let (ax, ay) = anchor.translate_coordinates(&toplevel, 0, 0)?;
        let allocation = anchor.allocation();
        let rect = view_frame(
            parent.bounds().size.height,
            ax,
            ay,
            allocation.width(),
            allocation.height(),
        );
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return None;
        }
        Some(NSRect::new(
            NSPoint::new(rect.x, rect.y),
            NSSize::new(rect.width, rect.height),
        ))
    }

    fn show(web: &WKWebView, parent: &NSView, rect: NSRect, suspended: bool) {
        place(web, rect);
        let same = unsafe { web.superview() }
            .is_some_and(|current| std::ptr::eq(current.as_ref(), parent));
        if !same {
            web.removeFromSuperview();
            parent.addSubview(web);
        }
        web.setHidden(suspended);
    }

    fn hide(web: &WKWebView) {
        web.setHidden(true);
        web.removeFromSuperview();
    }

    fn place(web: &WKWebView, rect: NSRect) {
        if std::env::var_os("EB_PANEL_DEBUG").is_some() {
            eprintln!(
                "preview view: x={} y={} w={} h={}",
                rect.origin.x, rect.origin.y, rect.size.width, rect.size.height
            );
        }
        let current = web.frame();
        if super::origin_move(
            current.size.width,
            current.size.height,
            rect.size.width,
            rect.size.height,
        ) && (current.origin.x - rect.origin.x).abs() < 0.5
            && (current.origin.y - rect.origin.y).abs() < 0.5
        {
            return;
        }
        web.setFrame(rect);
    }

    /// The page defines `window.__ebPreviewWake` to poll immediately. The
    /// string is not a document payload.
    fn poke(web: &WKWebView) {
        let script = NSString::from_str("if (window.__ebPreviewWake) window.__ebPreviewWake()");
        unsafe {
            web.evaluateJavaScript_completionHandler(&script, None);
        }
    }

    fn load(web: &WKWebView, url: &str) -> bool {
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            eprintln!("[evobuddy] preview URL was rejected");
            return false;
        };
        let request = NSURLRequest::requestWithURL(&url);
        unsafe {
            web.loadRequest(&request);
        }
        true
    }

    /// GDK does not return +1. The pointer is borrowed for this call only.
    fn return_key(anchor: &gtk::Widget) {
        let Some(toplevel) = anchor.toplevel().and_downcast::<gtk::Window>() else {
            return;
        };
        let Some(gdk_window) = toplevel.window() else {
            return;
        };
        let native: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
        unsafe {
            let nswindow = gdk_quartz_window_get_nswindow(native as *mut std::ffi::c_void);
            let nsview = gdk_quartz_window_get_nsview(native as *mut std::ffi::c_void);
            if nswindow.is_null() {
                return;
            }
            (*nswindow).makeKeyAndOrderFront(None);
            if !nsview.is_null() {
                (*nswindow).makeFirstResponder(Some(&*nsview));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::view_frame;

    #[test]
    fn view_frame_sits_below_the_header_in_an_unflipped_view() {
        let frame = view_frame(800.0, 12, 64, 400, 500);
        assert_eq!(frame.x, 12.0);
        assert_eq!(frame.y, 236.0);
        assert_eq!(frame.width, 400.0);
        assert_eq!(frame.height, 500.0);
    }

    #[test]
    fn a_pure_move_does_not_reset_the_frame() {
        assert!(super::origin_move(400.0, 300.0, 400.0, 300.0));
        assert!(super::origin_move(400.2, 300.0, 400.0, 300.0));
        assert!(!super::origin_move(400.0, 300.0, 380.0, 300.0));
    }
}
