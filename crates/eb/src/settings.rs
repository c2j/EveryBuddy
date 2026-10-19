//! Experimental Settings → Plugins surface.
//!
//! The native app itself has no JavaScript runtime. This window drives a real
//! `rutis-host` process with a generated `rutis.json`; rows are published dsh
//! host/Cordis packages and are added/removed at runtime. `--watch-config`
//! makes Apply a reload rather than a rebuild or an app restart.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gtk::prelude::*;

use crate::widgets::{class, flat_button, text, vbox};

#[derive(Clone, Copy)]
struct Plugin {
    id: &'static str,
    package: &'static str,
    title: &'static str,
    description: &'static str,
}

/// Host-side dsh packages known to load without a host-provided service.
const FS_BRIDGE: &str = include_str!("../../../experiments/dsh-runtime-settings/fs-bridge.mjs");
const FS_BRIDGE_PORT: u16 = 39471;

const CATALOG: &[Plugin] = &[
    Plugin {
        id: "invariants",
        package: "@deepseek-ai/dsh-invariants",
        title: "Invariants",
        description: "Core dsh invariants registry",
    },
    Plugin {
        id: "credentials",
        package: "@deepseek-ai/dsh-credentials-local",
        title: "Credentials",
        description: "Local credential provider",
    },
    Plugin {
        id: "fs",
        package: "@deepseek-ai/dsh-fs-local",
        title: "Filesystem",
        description: "Local filesystem service",
    },
    Plugin {
        id: "jobs",
        package: "@deepseek-ai/dsh-jobs-local",
        title: "Jobs",
        description: "Local background job registry",
    },
    Plugin {
        id: "commands",
        package: "@deepseek-ai/dsh-commands",
        title: "Commands",
        description: "Registry only; needs a command UI adapter",
    },
];

struct Controller {
    selected: RefCell<HashSet<&'static str>>,
    child: RefCell<Option<Child>>,
    files_windows: RefCell<Vec<gtk::Window>>,
    fs_running: Cell<bool>,
    log: Arc<Mutex<Vec<String>>>,
    config_path: PathBuf,
}

impl Controller {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            selected: RefCell::new(HashSet::new()),
            child: RefCell::new(None),
            files_windows: RefCell::new(Vec::new()),
            fs_running: Cell::new(false),
            log: Arc::new(Mutex::new(Vec::new())),
            config_path: std::env::temp_dir().join(format!(
                "evobuddy-dsh-plugins-{}.json",
                std::process::id()
            )),
        })
    }

    fn toggle(&self, id: &'static str, on: bool) {
        if on {
            self.selected.borrow_mut().insert(id);
        } else {
            self.selected.borrow_mut().remove(id);
        }
    }

    fn apply(&self) -> Result<(), String> {
        if std::env::var_os("EB_SETTINGS_DEBUG").is_some() {
            eprintln!("settings: apply selected={:?}", self.selected.borrow());
        }
        let selected: Vec<&Plugin> = CATALOG
            .iter()
            .filter(|plugin| self.selected.borrow().contains(plugin.id))
            .collect();
        if selected.is_empty() {
            self.stop();
            return Ok(());
        }
        if !self.selected.borrow().contains("fs") {
            self.set_fs_running(false);
        }

        let project = std::env::var("EVOBUDDY_DSH_NODE_PROJECT").unwrap_or_else(|_| {
            "/tmp/rutis/crates/rutis-dsh/dsh".to_string()
        });
        if !PathBuf::from(&project).join("package.json").is_file() {
            return Err(format!(
                "missing dsh npm project at {project}; set EVOBUDDY_DSH_NODE_PROJECT"
            ));
        }

        let mut rows: Vec<serde_json::Value> = selected
            .iter()
            .map(|plugin| {
                let mut row = serde_json::json!({
                    "id": plugin.id,
                    "name": plugin.package,
                });
                if plugin.id == "fs" {
                    let cwd = std::env::var("EVOBUDDY_DSH_WORKSPACE")
                        .or_else(|_| std::env::current_dir().map(|path| path.display().to_string()))
                        .unwrap_or_else(|_| ".".to_string());
                    row["config"] = serde_json::json!({ "cwd": cwd });
                }
                row
            })
            .collect();
        if self.selected.borrow().contains("fs") {
            let bridge = std::env::temp_dir().join(format!(
                "evobuddy-fs-bridge-{}.mjs",
                std::process::id()
            ));
            std::fs::write(&bridge, FS_BRIDGE)
                .map_err(|error| format!("cannot write fs bridge: {error}"))?;
            rows.push(serde_json::json!({
                "id": "fs_bridge",
                "name": bridge.display().to_string(),
            }));
        }
        let config = serde_json::json!({
            "format": 1,
            "runtimes": { "node": { "project": project } },
            "rows": rows,
        });
        std::fs::write(
            &self.config_path,
            serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("cannot write {}: {error}", self.config_path.display()))?;

        if self.child.borrow().is_some() {
            // --watch-config notices the rewrite and reconciles the rows.
            return Ok(());
        }

        let host = std::env::var("EVOBUDDY_RUTIS_HOST").unwrap_or_else(|_| {
            let built = "/tmp/rutis/target/debug/rutis-host";
            if PathBuf::from(built).is_file() {
                built.to_string()
            } else {
                "rutis-host".to_string()
            }
        });
        let mut command = Command::new(&host);
        command
            .arg("run")
            .arg("--watch-config")
            .arg(&self.config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Ok(path) = std::env::var("EVOBUDDY_NODE_BIN") {
            if let Some(dir) = PathBuf::from(path).parent() {
                let old = std::env::var("PATH").unwrap_or_default();
                command.env("PATH", format!("{}:{old}", dir.display()));
            }
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start {host}: {error}"))?;
        if let Some(stdout) = child.stdout.take() {
            let log = self.log.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Ok(mut log) = log.lock() {
                        log.push(line);
                        if log.len() > 200 {
                            log.remove(0);
                        }
                    }
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let log = self.log.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Ok(mut log) = log.lock() {
                        log.push(format!("! {line}"));
                        if log.len() > 200 {
                            log.remove(0);
                        }
                    }
                }
            });
        }
        *self.child.borrow_mut() = Some(child);
        if std::env::var_os("EB_SETTINGS_DEBUG").is_some() {
            eprintln!("settings: started {host} with {}", self.config_path.display());
        }
        Ok(())
    }

    fn stop(&self) {
        self.fs_running.set(false);
        self.close_files();
        if let Some(mut child) = self.child.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn track_files_window(&self, window: &gtk::Window) {
        self.files_windows.borrow_mut().push(window.clone());
    }

    fn close_files(&self) {
        for window in self.files_windows.borrow_mut().drain(..) {
            window.close();
        }
    }

    fn set_fs_running(&self, running: bool) {
        self.fs_running.set(running);
        if !running {
            self.close_files();
        }
    }

    fn drain_log(&self) -> Vec<String> {
        self.log
            .lock()
            .map(|mut log| std::mem::take(&mut *log))
            .unwrap_or_default()
    }
}

fn http_get(path: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", FS_BRIDGE_PORT))
        .map_err(|error| format!("fs bridge is not running: {error}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| error.to_string())?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "malformed fs bridge response".to_string())?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(body.to_string());
    }
    Ok(body.to_string())
}

fn encode_path(path: &str) -> String {
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[derive(Clone)]
struct FilesView {
    path: Rc<RefCell<String>>,
    path_label: gtk::Label,
    list: gtk::Box,
    content: gtk::TextView,
}

impl FilesView {
    fn refresh(self: &Rc<Self>) {
        let path = self.path.borrow().clone();
        let encoded = encode_path(&path);
        let body = match http_get(&format!("/list?path={encoded}")) {
            Ok(body) => body,
            Err(error) => {
                self.set_message(&format!("Filesystem unavailable: {error}"));
                return;
            }
        };
        let entries: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap_or_default();
        self.path_label.set_text(&path);
        for child in self.list.children() {
            self.list.remove(&child);
        }
        if entries.is_empty() {
            self.list.pack_start(
                &crate::widgets::text("This directory is empty.", "eb-files-empty"),
                false,
                false,
                0,
            );
        }
        for entry in entries {
            let name = entry
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("?")
                .to_string();
            let is_dir = entry
                .get("type")
                .and_then(|value| value.as_str())
                .map(|value| value == "directory")
                .unwrap_or(false);
            let button = flat_button("eb-files-row");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.pack_start(
                &crate::icons::named(if is_dir { "library" } else { "code" }, 16, "eb-files-icon"),
                false,
                false,
                0,
            );
            row.pack_start(
                &crate::widgets::text(&name, "eb-files-name"),
                true,
                true,
                0,
            );
            button.add(&row);
            let this = self.clone();
            let entry_path = if path == "." {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            button.connect_clicked(move |_| {
                if is_dir {
                    *this.path.borrow_mut() = entry_path.clone();
                    this.refresh();
                } else {
                    let body = match http_get(&format!("/read?path={}", encode_path(&entry_path))) {
                        Ok(body) => body,
                        Err(error) => format!("Cannot read {entry_path}: {error}"),
                    };
                    if let Some(buffer) = this.content.buffer() {
                        buffer.set_text(&body);
                    }
                }
            });
            self.list.pack_start(&button, false, false, 0);
        }
        self.list.show_all();
    }

    fn set_message(&self, message: &str) {
        for child in self.list.children() {
            self.list.remove(&child);
        }
        self.list.pack_start(
            &crate::widgets::text(message, "eb-files-empty"),
            false,
            false,
            0,
        );
        self.list.show_all();
    }
}

fn capture_window(window: &gtk::Window, path: &str) {
    window.size_allocate(&window.allocation());
    let allocation = window.allocation();
    let (width, height) = (allocation.width(), allocation.height());
    let Ok(surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height)
    else {
        return;
    };
    let Ok(context) = gtk::cairo::Context::new(&surface) else {
        return;
    };
    let widget: gtk::Widget = window.clone().upcast();
    let _ = widget.draw(&context);
    if let Ok(mut file) = std::fs::File::create(path) {
        let _ = surface.write_to_png(&mut file);
        eprintln!("settings: wrote {path}");
    }
}

fn open_files(controller: Rc<Controller>) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Files — dsh fs-local");
    window.set_default_size(560, 640);
    let root = vbox(0);
    class(&root, "eb-files");
    window.add(&root);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    class(&header, "eb-files-header");
    let title = crate::widgets::text("Files", "eb-files-title");
    header.pack_start(&title, false, false, 0);
    let path_label = crate::widgets::text(".", "eb-files-path");
    header.pack_start(&path_label, true, true, 0);
    let up = flat_button("eb-files-up");
    up.add(&crate::widgets::text("Up", "eb-files-up-label"));
    header.pack_end(&up, false, false, 0);
    root.pack_start(&header, false, false, 0);

    let list = vbox(0);
    let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.add(&list);
    scroller.set_vexpand(true);
    root.pack_start(&scroller, true, true, 0);

    let content = gtk::TextView::new();
    content.set_editable(false);
    content.set_monospace(true);
    class(&content, "eb-files-content");
    let content_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    content_scroll.set_min_content_height(180);
    content_scroll.add(&content);
    root.pack_start(&content_scroll, false, false, 0);

    let view = Rc::new(FilesView {
        path: Rc::new(RefCell::new(".".to_string())),
        path_label,
        list,
        content,
    });
    {
        let view = view.clone();
        up.connect_clicked(move |_| {
            let mut path = view.path.borrow_mut();
            if path.as_str() != "." {
                if let Some((parent, _)) = path.rsplit_once('/') {
                    *path = parent.to_string();
                } else {
                    *path = ".".to_string();
                }
            }
            drop(path);
            view.refresh();
        });
    }
    view.refresh();
    window.show_all();
    controller.track_files_window(&window);
    if let Ok(path) = std::env::var("EB_SETTINGS_FILES_SHOT") {
        let window = window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            capture_window(&window, &path);
        });
    }
}

/// Open the Settings window. The window owns the controller; closing it stops
/// the sidecar so the experiment leaves no process behind.
pub fn open() {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Settings — Plugins");
    window.set_default_size(900, 680);
    let root = vbox(0);
    class(&root, "eb-settings");
    window.add(&root);

    let header = vbox(4);
    class(&header, "eb-settings-header");
    header.pack_start(&text("Plugins", "eb-settings-title"), false, false, 0);
    header.pack_start(
        &text(
            "Runtime dsh host plugins via rutis-loader. Apply rewrites rutis.json; no rebuild. Host services only — no UI bridge yet.",
            "eb-settings-subtitle",
        ),
        false,
        false,
        0,
    );
    root.pack_start(&header, false, false, 0);

    let controller = Controller::new();
    let rows = vbox(2);
    class(&rows, "eb-settings-rows");
    let mut status_labels: Vec<(&'static str, gtk::Label)> = Vec::new();
    let auto_select = std::env::var("EB_SETTINGS_SELECT").unwrap_or_default();
    for plugin in CATALOG {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        class(&row, "eb-settings-row");
        let check = gtk::CheckButton::new();
        let id = plugin.id;
        let controller_for_toggle = controller.clone();
        check.connect_toggled(move |check| {
            controller_for_toggle.toggle(id, check.is_active());
        });
        check.set_active(auto_select.split(',').any(|value| value.trim() == plugin.id));
        row.pack_start(&check, false, false, 0);

        let copy = vbox(2);
        copy.pack_start(&text(plugin.title, "eb-settings-name"), false, false, 0);
        copy.pack_start(
            &text(plugin.description, "eb-settings-description"),
            false,
            false,
            0,
        );
        row.pack_start(&copy, true, true, 0);

        let status = text("not loaded", "eb-settings-status");
        row.pack_end(&status, false, false, 0);
        status_labels.push((plugin.id, status));
        rows.pack_start(&row, false, false, 0);
    }
    root.pack_start(&rows, false, false, 0);

    let ui_header = vbox(2);
    class(&ui_header, "eb-settings-ui-header");
    ui_header.pack_start(
        &text("UI plugin previews", "eb-settings-title-small"),
        false,
        false,
        0,
    );
    ui_header.pack_start(
        &text(
            "Client bundles rendered in a WebKit component host. These are previews, not in-app panes yet.",
            "eb-settings-description",
        ),
        false,
        false,
        0,
    );
    root.pack_start(&ui_header, false, false, 0);

    let ui_rows = vbox(2);
    class(&ui_rows, "eb-settings-rows");
    for (id, title, description, index) in [
        (
            "ui-theme",
            "Theme",
            "Real ui-theme bundle: Appearance and text size",
            "experiments/ui-theme-webview/index.html",
        ),
        (
            "ui-message-feedback",
            "Message feedback",
            "Real ui-message-feedback bundle: Like/Dislike and dialog",
            "experiments/ui-feedback-webview/index.html",
        ),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        class(&row, "eb-settings-row");
        let copy = vbox(2);
        copy.pack_start(&text(title, "eb-settings-name"), false, false, 0);
        copy.pack_start(&text(description, "eb-settings-description"), false, false, 0);
        row.pack_start(&copy, true, true, 0);
        let open = flat_button("eb-settings-preview");
        open.add(&text("Open preview", "eb-settings-preview-label"));
        #[cfg(target_os = "macos")]
        {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(index);
            open.connect_clicked(move |_| {
                crate::theme_view::open(&path.display().to_string());
            });
        }
        row.pack_end(&open, false, false, 0);
        if std::env::var("EB_SETTINGS_UI_PREVIEW").ok().as_deref() == Some(id) {
            let open = open.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                open.clicked();
            });
        }
        ui_rows.pack_start(&row, false, false, 0);
    }
    root.pack_start(&ui_rows, false, false, 0);

    let bridge_header = vbox(2);
    class(&bridge_header, "eb-settings-ui-header");
    bridge_header.pack_start(
        &text("Live dsh bridge", "eb-settings-title-small"),
        false,
        false,
        0,
    );
    bridge_header.pack_start(
        &text(
            "Run rutis 0.12's real agent + messageFeedback service on 127.0.0.1:39472.",
            "eb-settings-description",
        ),
        false,
        false,
        0,
    );
    root.pack_start(&bridge_header, false, false, 0);

    let bridge_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    class(&bridge_row, "eb-settings-row");
    let bridge_status = text("stopped", "eb-settings-description");
    bridge_row.pack_start(&bridge_status, true, true, 0);
    let bridge_button = flat_button("eb-settings-preview");
    bridge_button.add(&text("Start bridge", "eb-settings-preview-label"));
    bridge_row.pack_end(&bridge_button, false, false, 0);
    root.pack_start(&bridge_row, false, false, 0);

    let bridge_child: Rc<RefCell<Option<Child>>> = Rc::new(RefCell::new(None));
    {
        let bridge_child = bridge_child.clone();
        let bridge_status = bridge_status.clone();
        let bridge_button_for_click = bridge_button.clone();
        bridge_button.connect_clicked(move |_| {
            let mut slot = bridge_child.borrow_mut();
            if let Some(mut child) = slot.take() {
                let _ = child.kill();
                let _ = child.wait();
                bridge_status.set_text("stopped");
                if let Some(label) = bridge_button_for_click.child().and_downcast::<gtk::Label>() {
                    label.set_text("Start bridge");
                }
                return;
            }
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let binary = std::env::var("EVOBUDDY_BRIDGE_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    root.join("vendor/rutis-0.12/target/debug/evobuddy-bridge")
                });
            match Command::new(&binary)
                .env("EVOBUDDY_LIVE_BRIDGE", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => {
                    *slot = Some(child);
                    bridge_status.set_text("running · http://127.0.0.1:39472");
                    if let Some(label) = bridge_button_for_click.child().and_downcast::<gtk::Label>() {
                        label.set_text("Stop bridge");
                    }
                }
                Err(error) => {
                    bridge_status.set_text(&format!("not built: {error}"));
                }
            }
        });
    }
    if std::env::var_os("EB_BRIDGE_AUTOSTART").is_some() {
        let bridge_button = bridge_button.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
            bridge_button.clicked();
        });
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    class(&actions, "eb-settings-actions");
    let apply = flat_button("eb-settings-apply");
    apply.add(&text("Apply & Reload", "eb-settings-apply-label"));
    actions.pack_start(&apply, false, false, 0);
    let stop = flat_button("eb-settings-stop");
    stop.add(&text("Stop", "eb-settings-stop-label"));
    actions.pack_start(&stop, false, false, 0);
    let files = flat_button("eb-settings-files");
    files.add(&text("Open Files", "eb-settings-files-label"));
    files.set_visible(false);
    actions.pack_start(&files, false, false, 0);
    root.pack_start(&actions, false, false, 0);

    let log_view = text("", "eb-settings-log");
    log_view.set_xalign(0.0);
    log_view.set_selectable(true);
    log_view.set_line_wrap(false);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(150);
    scroll.add(&log_view);
    root.pack_start(&scroll, true, true, 0);

    {
        let controller = controller.clone();
        let status_labels = status_labels.clone();
        let log_view = log_view.clone();
        let files = files.clone();
        apply.connect_clicked(move |_| {
            match controller.apply() {
                Ok(()) => {
                    for (id, label) in &status_labels {
                        let selected = controller.selected.borrow().contains(id);
                        label.set_text(if selected { "starting…" } else { "disabled" });
                    }
                    files.set_visible(controller.selected.borrow().contains("fs"));
                }
                Err(error) => {
                    if std::env::var_os("EB_SETTINGS_DEBUG").is_some() {
                        eprintln!("settings: apply error: {error}");
                    }
                    log_view.set_text(&format!("error: {error}"));
                }
            }
        });
    }
    {
        let controller = controller.clone();
        let status_labels = status_labels.clone();
        let files = files.clone();
        stop.connect_clicked(move |_| {
            controller.stop();
            for (_, label) in &status_labels {
                label.set_text("stopped");
            }
            files.set_visible(false);
        });
    }

    {
        let files = files.clone();
        let controller = controller.clone();
        files.connect_clicked(move |_| open_files(controller.clone()));
    }
    if std::env::var_os("EB_SETTINGS_FILES").is_some() {
        let files = files.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            files.clicked();
        });
    }

    let alive = Rc::new(Cell::new(true));
    {
        let alive = alive.clone();
        let controller = controller.clone();
        window.connect_destroy(move |_| {
            alive.set(false);
            controller.stop();
        });
    }
    let timer = gtk::glib::timeout_add_local(std::time::Duration::from_millis(500), {
        let controller = controller.clone();
        let log_view = log_view.clone();
        let status_labels = status_labels.clone();
        let files = files.clone();
        let alive = alive.clone();
        move || {
            if !alive.get() {
                return gtk::glib::ControlFlow::Break;
            }
            let lines = controller.drain_log();
            if !lines.is_empty() {
                let mut text = log_view.text().to_string();
                for line in lines {
                    if std::env::var_os("EB_SETTINGS_DEBUG").is_some() {
                        eprintln!("settings: {line}");
                    }
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&line);
                    if let Some(rest) = line.strip_prefix("fs: ") {
                        controller.set_fs_running(rest == "running");
                    }
                    for (id, label) in &status_labels {
                        if let Some(rest) = line.strip_prefix(&format!("{id}: ")) {
                            label.set_text(rest);
                        }
                    }
                }
                log_view.set_text(&text);
            }
            files.set_visible(controller.fs_running.get());
            gtk::glib::ControlFlow::Continue
        }
    });
    let _ = timer;

    window.show_all();
    if let Ok(path) = std::env::var("EB_SETTINGS_SHOT") {
        let window = window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
            window.size_allocate(&window.allocation());
            let allocation = window.allocation();
            let (width, height) = (allocation.width(), allocation.height());
            let Ok(surface) = gtk::cairo::ImageSurface::create(
                gtk::cairo::Format::ARgb32,
                width,
                height,
            ) else {
                return;
            };
            let Ok(context) = gtk::cairo::Context::new(&surface) else {
                return;
            };
            let widget: gtk::Widget = window.clone().upcast();
            let _ = widget.draw(&context);
            if let Ok(mut file) = std::fs::File::create(&path) {
                let _ = surface.write_to_png(&mut file);
                eprintln!("settings: wrote {path}");
            }
        });
    }
    if !auto_select.is_empty() {
        let apply = apply.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
            apply.clicked();
        });
    }
}
