//! Windows preview host.
//!
//! One WebView2 child is parented to a dedicated `GtkDrawingArea` inside
//! `detail.body`. The area is hidden, not destroyed, and the next Read reuses
//! it: `build_as_child` runs only for the first successful embed.
//!
//! Bounds are logical pixels of that area, origin at its top-left. wry
//! converts them with the child HWND's DPI. Quartz points and a screen-space
//! flip are not involved.
//!
//! `WebView2Loader` comes from `webview2-com-sys`, which wry 0.57.1 already
//! links. On the gnu target that crate imports `WebView2Loader.dll`. On msvc
//! it links `WebView2LoaderStatic.lib`. This module does not enable wry's
//! `os-webview` feature. Runtime presence is
//! `GetAvailableCoreWebView2BrowserVersionString` from that same loader.

/// Logical rectangle of the webview inside the anchor HWND.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalChildBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The child fills the DrawingArea. An empty allocation is not a frame.
pub fn logical_child_bounds(width: i32, height: i32) -> Option<LogicalChildBounds> {
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(LogicalChildBounds {
        x: 0.0,
        y: 0.0,
        width: f64::from(width),
        height: f64::from(height),
    })
}

/// An empty or blank version string means the Evergreen runtime is absent.
pub fn runtime_version_present(version: &str) -> bool {
    !version.trim().is_empty()
}

/// Shown when `GetAvailableCoreWebView2BrowserVersionString` reports no runtime.
/// The shell has no status bar; this goes to stderr with the other fallbacks.
pub fn runtime_install_hint() -> &'static str {
    "WebView2 Runtime is missing. Install the Evergreen Bootstrapper (https://go.microsoft.com/fwlink/p/?LinkId=2124703) and open the file again. The native view is shown until then."
}

/// The embed failure `present` recorded, if the shell has not taken it yet.
pub fn take_fallback() -> Option<String> {
    #[cfg(windows)]
    {
        host::take_fallback()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Called when a fallback is discovered after `refresh` has already returned,
/// for example from a later realize. The synchronous path uses
/// [`take_fallback`] instead.
pub fn set_late_fallback(handler: std::rc::Rc<dyn Fn()>) {
    #[cfg(windows)]
    host::set_late_fallback(handler);
    #[cfg(not(windows))]
    {
        let _ = handler;
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn attach(anchor: gtk::DrawingArea) {
    #[cfg(windows)]
    host::attach(anchor);
    #[cfg(not(windows))]
    {
        let _ = anchor;
    }
}

pub fn present() {
    #[cfg(windows)]
    host::present();
}

pub fn dismiss() {
    #[cfg(windows)]
    host::dismiss();
}

pub fn reposition() {
    #[cfg(windows)]
    host::reposition();
}

/// Ask the loaded page to pull `/session` now. The script carries no document
/// bytes. A repeat of the same session version does nothing.
pub fn wake() {
    #[cfg(windows)]
    host::wake();
}

#[cfg(windows)]
mod host {
    use std::cell::{Cell, RefCell};
    use std::num::NonZeroIsize;
    use std::rc::Rc;

    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};

    use super::{logical_child_bounds, runtime_install_hint, runtime_version_present};

    #[link(name = "gdk-3")]
    extern "C" {
        fn gdk_win32_window_get_handle(window: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(pv: *mut std::ffi::c_void);
    }

    const WAKE_SCRIPT: &str = "if (window.__ebPreviewWake) window.__ebPreviewWake()";

    struct Host {
        anchor: gtk::DrawingArea,
        web: Option<wry::WebView>,
        visible: bool,
        woken: Option<u64>,
        realize_hooked: bool,
    }

    struct ChildWindow {
        hwnd: NonZeroIsize,
    }

    impl HasWindowHandle for ChildWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, raw_window_handle::HandleError> {
            let raw = RawWindowHandle::Win32(Win32WindowHandle::new(self.hwnd));
            unsafe { Ok(WindowHandle::borrow_raw(raw)) }
        }
    }

    thread_local! {
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
        static FALLBACK: RefCell<Option<String>> = const { RefCell::new(None) };
        static RUNTIME: Cell<Option<bool>> = const { Cell::new(None) };
        static LATE: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    }

    pub fn set_late_fallback(handler: Rc<dyn Fn()>) {
        LATE.with(|slot| *slot.borrow_mut() = Some(handler));
    }

    pub fn take_fallback() -> Option<String> {
        FALLBACK.with(|slot| slot.borrow_mut().take())
    }

    pub fn attach(anchor: gtk::DrawingArea) {
        anchor.connect_size_allocate(|anchor, allocation| {
            if logical_child_bounds(allocation.width(), allocation.height()).is_none() {
                return;
            }
            if !visible_web() {
                return;
            }
            apply_bounds_of(anchor);
        });
        HOST.with(|slot| {
            *slot.borrow_mut() = Some(Host {
                anchor,
                web: None,
                visible: false,
                woken: None,
                realize_hooked: false,
            });
        });
    }

    pub fn present() {
        let Some(anchor) = anchor() else {
            return;
        };
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut() {
                host.visible = true;
            }
        });
        if !runtime_ready() {
            request_fallback(runtime_install_hint());
            return;
        }
        if has_web() {
            show_existing(&anchor);
            return;
        }
        let Some(url) = crate::preview_web_server::shared_preview_url() else {
            request_fallback("preview server did not start; showing the native view");
            return;
        };
        let Some(hwnd) = hwnd_of(&anchor) else {
            hook_realize(&anchor);
            return;
        };
        let bounds = current_bounds(&anchor);
        let mut builder = wry::WebViewBuilder::new().with_url(&url).with_visible(true);
        if let Some(bounds) = bounds {
            builder = builder.with_bounds(wry_rect(bounds));
        }
        match builder.build_as_child(&ChildWindow { hwnd }) {
            Ok(web) => {
                // The controller exists when `build_as_child` returns. Set the
                // logical bounds once here, and again on every later allocate.
                if let Some(bounds) = current_bounds(&anchor) {
                    let _ = web.set_bounds(wry_rect(bounds));
                }
                let _ = web.set_visible(true);
                HOST.with(|slot| {
                    if let Some(host) = slot.borrow_mut().as_mut() {
                        host.web = Some(web);
                        host.visible = true;
                    }
                });
            }
            Err(error) => {
                request_fallback(&format!(
                    "preview webview did not start ({error}); showing the native view"
                ));
            }
        }
    }

    pub fn dismiss() {
        HOST.with(|slot| {
            let Some(host) = slot.borrow_mut().as_mut() else {
                return;
            };
            host.visible = false;
            if let Some(web) = host.web.as_ref() {
                let _ = web.set_visible(false);
            }
        });
    }

    pub fn reposition() {
        let Some(anchor) = anchor() else {
            return;
        };
        if !visible_web() {
            return;
        }
        apply_bounds_of(&anchor);
    }

    pub fn wake() {
        let Some(version) =
            crate::preview_web_server::session_view_shared().map(|view| view.version)
        else {
            return;
        };
        HOST.with(|slot| {
            let Some(host) = slot.borrow_mut().as_mut() else {
                return;
            };
            if host.web.is_none() || !host.visible || host.woken == Some(version) {
                return;
            }
            host.woken = Some(version);
            if let Some(web) = host.web.as_ref() {
                let _ = web.evaluate_script(WAKE_SCRIPT);
            }
        });
    }

    fn anchor() -> Option<gtk::DrawingArea> {
        HOST.with(|slot| slot.borrow().as_ref().map(|host| host.anchor.clone()))
    }

    fn has_web() -> bool {
        HOST.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|host| host.web.is_some())
        })
    }

    fn visible_web() -> bool {
        HOST.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|host| host.visible && host.web.is_some())
        })
    }

    fn show_existing(anchor: &gtk::DrawingArea) {
        HOST.with(|slot| {
            let Some(host) = slot.borrow().as_ref() else {
                return;
            };
            let Some(web) = host.web.as_ref() else {
                return;
            };
            if let Some(bounds) = current_bounds(anchor) {
                let _ = web.set_bounds(wry_rect(bounds));
            }
            let _ = web.set_visible(true);
        });
    }

    fn apply_bounds_of(anchor: &gtk::DrawingArea) {
        let Some(bounds) = current_bounds(anchor) else {
            return;
        };
        HOST.with(|slot| {
            let Some(host) = slot.borrow().as_ref() else {
                return;
            };
            if !host.visible {
                return;
            }
            if let Some(web) = host.web.as_ref() {
                if std::env::var_os("EB_PANEL_DEBUG").is_some() {
                    eprintln!(
                        "preview view: x={} y={} w={} h={}",
                        bounds.x, bounds.y, bounds.width, bounds.height
                    );
                }
                let _ = web.set_bounds(wry_rect(bounds));
            }
        });
    }

    fn current_bounds(anchor: &gtk::DrawingArea) -> Option<super::LogicalChildBounds> {
        let allocation = anchor.allocation();
        logical_child_bounds(allocation.width(), allocation.height())
    }

    fn wry_rect(bounds: super::LogicalChildBounds) -> wry::Rect {
        wry::Rect {
            position: wry::dpi::LogicalPosition::new(bounds.x, bounds.y).into(),
            size: wry::dpi::LogicalSize::new(bounds.width, bounds.height).into(),
        }
    }

    fn hwnd_of(anchor: &gtk::DrawingArea) -> Option<NonZeroIsize> {
        if anchor.window().is_none() {
            anchor.realize();
        }
        let window = anchor.window()?;
        let native: *mut gtk::gdk::ffi::GdkWindow = window.to_glib_none().0;
        let raw = unsafe { gdk_win32_window_get_handle(native as *mut std::ffi::c_void) };
        NonZeroIsize::new(raw as isize)
    }

    fn hook_realize(anchor: &gtk::DrawingArea) {
        let hooked = HOST.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|host| host.realize_hooked)
        });
        if hooked {
            return;
        }
        HOST.with(|slot| {
            if let Some(host) = slot.borrow_mut().as_mut() {
                host.realize_hooked = true;
            }
        });
        anchor.connect_realize(|_| present());
    }

    fn runtime_ready() -> bool {
        if let Some(known) = RUNTIME.with(|cell| cell.get()) {
            return known;
        }
        let ready = query_runtime();
        RUNTIME.with(|cell| cell.set(Some(ready)));
        ready
    }

    fn query_runtime() -> bool {
        use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
        let mut version = windows_core::PWSTR::null();
        let status = unsafe {
            GetAvailableCoreWebView2BrowserVersionString(windows_core::PCWSTR::null(), &mut version)
        };
        let text = if version.is_null() {
            String::new()
        } else {
            let text = unsafe { version.to_string() }.unwrap_or_default();
            unsafe { CoTaskMemFree(version.0.cast()) };
            text
        };
        status.is_ok() && runtime_version_present(&text)
    }

    fn request_fallback(message: &str) {
        FALLBACK.with(|slot| *slot.borrow_mut() = Some(message.to_string()));
        gtk::glib::idle_add_once(|| {
            LATE.with(|slot| {
                if let Some(handler) = slot.borrow().as_ref().cloned() {
                    handler();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_bounds_fill_the_anchor_from_its_origin() {
        let bounds = logical_child_bounds(400, 500).unwrap();
        assert_eq!(bounds.x, 0.0);
        assert_eq!(bounds.y, 0.0);
        assert_eq!(bounds.width, 400.0);
        assert_eq!(bounds.height, 500.0);
    }

    #[test]
    fn an_empty_anchor_is_not_a_frame() {
        assert!(logical_child_bounds(0, 10).is_none());
        assert!(logical_child_bounds(10, 0).is_none());
        assert!(logical_child_bounds(-1, 10).is_none());
    }

    #[test]
    fn a_blank_runtime_version_is_missing() {
        assert!(!runtime_version_present(""));
        assert!(!runtime_version_present("   "));
        assert!(runtime_version_present("120.0.2210.91"));
    }

    #[test]
    fn the_missing_runtime_hint_names_the_bootstrapper() {
        let hint = runtime_install_hint();
        assert!(hint.contains("https://go.microsoft.com/fwlink/p/?LinkId=2124703"));
        assert!(hint.ends_with("The native view is shown until then."));
    }
}
