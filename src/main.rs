// On Windows, build as a GUI (rather than console) application so launching the
// binary does not open an extra console window. Ignored on other platforms.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod notebook;

use gtk::prelude::*;

use notebook::Notebook;

/// Hint shown in every editor. It doubles as a check that CJK glyphs render.
const EDITOR_HINT: &str = "在此输入内容（支持中文输入）";

fn main() {
    ensure_gtk_im_modules();

    let application = gtk::Application::new(
        Some("com.github.gtk-rs.examples.notebook"),
        Default::default(),
    );

    application.connect_activate(build_ui);

    application.run();
}

/// Point GTK at its input-method module cache, so IME input works.
///
/// GTK derives its installation prefix from the path of the running
/// executable. A cargo binary such as `target/debug/notebook` lives outside the
/// GTK install tree, so GTK cannot find `immodules.cache`: its input-method
/// table is empty, it silently falls back to the "simple" context, and typing
/// with an IME (Chinese, Japanese, Korean, ...) does nothing.
///
/// We locate the *loaded* libgtk with `dladdr`, derive its `lib/` directory and
/// set `GTK_IM_MODULE_FILE` to the matching cache. Any user-supplied setting
/// (`GTK_IM_MODULE_FILE` or `GTK_EXE_PREFIX`) takes precedence.
#[cfg(target_os = "macos")]
fn ensure_gtk_im_modules() {
    use std::ffi::{c_char, CStr};
    use std::path::Path;

    if std::env::var_os("GTK_IM_MODULE_FILE").is_some()
        || std::env::var_os("GTK_EXE_PREFIX").is_some()
    {
        return;
    }

    // SAFETY: `dlsym`/`dladdr` only resolve addresses and write into `info`;
    // `dli_fname` points at loader-owned storage that stays valid afterwards.
    let cache = unsafe {
        let symbol = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"gtk_init_check".as_ptr() as *const c_char,
        );
        if symbol.is_null() {
            return;
        }

        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(symbol, &mut info) == 0 || info.dli_fname.is_null() {
            return;
        }

        let libgtk = CStr::from_ptr(info.dli_fname)
            .to_string_lossy()
            .into_owned();
        let Some(libdir) = Path::new(&libgtk).parent() else {
            return;
        };
        libdir.join("gtk-3.0").join("3.0.0").join("immodules.cache")
    };

    if cache.is_file() {
        std::env::set_var("GTK_IM_MODULE_FILE", cache);
    }
}

/// On other platforms GTK locates its input-method modules on its own.
#[cfg(not(target_os = "macos"))]
fn ensure_gtk_im_modules() {}

fn build_ui(application: &gtk::Application) {
    let window = gtk::ApplicationWindow::new(application);

    window.set_title("Notebook");
    window.set_position(gtk::WindowPosition::Center);
    window.set_default_size(640, 480);

    let mut notebook = Notebook::new();

    for i in 1..4 {
        let title = format!("sheet {i}");
        notebook.create_tab(&title, build_editor().upcast());
    }

    window.add(&notebook.notebook);
    window.show_all();
}

/// One editor page: a scrollable, wrapping multi-line text view.
///
/// CJK input itself needs no widget-specific code: once GTK's `im-quartz`
/// module is reachable (see `ensure_gtk_im_modules`) the text view accepts IME
/// composition, and fontconfig supplies the CJK fonts for rendering.
fn build_editor() -> gtk::ScrolledWindow {
    let buffer = gtk::TextBuffer::new(gtk::TextTagTable::NONE);
    buffer.set_text(EDITOR_HINT);

    let text_view = gtk::TextView::with_buffer(&buffer);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_left_margin(8);
    text_view.set_right_margin(8);
    text_view.set_top_margin(8);
    text_view.set_bottom_margin(8);

    let scrolled = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.add(&text_view);
    scrolled
}
