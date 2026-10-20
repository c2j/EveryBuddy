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

fn model_section() -> gtk::Box {
    let page = vbox(8);
    class(&page, "eb-settings-model-page");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.pack_start(&text("Provider keys", "eb-settings-title-small"), false, false, 0);
    let open_config = flat_button("eb-settings-preview");
    open_config.add(&text("Open config file", "eb-settings-preview-label"));
    open_config.connect_clicked(move |_| {
        let path = crate::models::config_path_display();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&path).spawn();
        #[cfg(not(target_os = "macos"))]
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    });
    header.pack_end(&open_config, false, false, 0);
    let dsh_web = flat_button("eb-settings-preview");
    dsh_web.add(&text("Open dsh Models", "eb-settings-preview-label"));
    dsh_web.connect_clicked(move |_| {
        let url = std::env::var("EVOBUDDY_DSH_WEB_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3080".into());
        #[cfg(target_os = "macos")]
        {
            crate::theme_view::open(&url);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
        }
    });
    header.pack_end(&dsh_web, false, false, 0);
    page.pack_start(&header, false, false, 0);
    page.pack_start(
        &text(
            "Fill in a provider's API key to use its models. Keys are write-only and stored under ~/.evobuddy. Open dsh Models uses the real dsh web plugin when the dsh web host is running.",
            "eb-settings-subtitle",
        ),
        false,
        false,
        0,
    );
    let list = vbox(6);
    page.pack_start(&list, false, false, 0);
    fill_model_rows(&page, &list);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let use_dsh = flat_button("eb-settings-apply");
    use_dsh.add(&text("Use dsh as default", "eb-settings-apply-label"));
    let status = text("", "eb-settings-status");
    actions.pack_start(&use_dsh, false, false, 0);
    actions.pack_start(&status, true, true, 0);
    page.pack_start(&actions, false, false, 0);
    use_dsh.connect_clicked(move |_| match crate::router::set_default(crate::router::DefaultRoute::Dsh) {
        Ok(()) => status.set_text("Saved. The next message uses the dsh runtime."),
        Err(error) => status.set_text(&format!("Save failed: {error}")),
    });
    page
}

fn clear_children(container: &gtk::Box) {
    for child in container.children() {
        container.remove(&child);
    }
}

fn fill_model_rows(page: &gtk::Box, list: &gtk::Box) {
    clear_children(list);
    let config = crate::models::load();
    for provider in &config.providers {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        class(&row, "eb-settings-provider-row");
        let name = text(&provider.id, "eb-settings-provider-name");
        row.pack_start(&name, false, false, 0);
        if provider.custom {
            let badge = text("custom", "eb-settings-provider-badge");
            row.pack_start(&badge, false, false, 0);
        }
        let dot = text(if crate::models::has_key(&provider.id) { "●" } else { "○" }, if crate::models::has_key(&provider.id) { "eb-settings-dot-on" } else { "eb-settings-dot-off" });
        row.pack_start(&dot, false, false, 0);
        let spacer = gtk::Label::new(None);
        spacer.set_hexpand(true);
        row.pack_start(&spacer, true, true, 0);
        let edit = flat_button("eb-settings-preview");
        edit.add(&text("Edit", "eb-settings-preview-label"));
        let page_for_edit = page.clone();
        let list_for_edit = list.clone();
        let provider_for_edit = provider.clone();
        edit.connect_clicked(move |_| {
            open_model_editor(&page_for_edit, &list_for_edit, Some(provider_for_edit.clone()));
        });
        row.pack_end(&edit, false, false, 0);
        let delete = flat_button("eb-settings-stop");
        delete.add(&text("Delete", "eb-settings-stop-label"));
        let page_for_delete = page.clone();
        let list_for_delete = list.clone();
        let id = provider.id.clone();
        delete.connect_clicked(move |_| {
            let mut config = crate::models::load();
            if crate::models::delete_provider(&mut config, &id).is_ok() {
                crate::bridge::stop();
                fill_model_rows(&page_for_delete, &list_for_delete);
            }
        });
        row.pack_end(&delete, false, false, 0);
        list.pack_start(&row, false, false, 0);
    }
    let add = flat_button("eb-settings-add-provider");
    add.add(&text("+ Add model provider", "eb-settings-add-provider-label"));
    let page_for_add = page.clone();
    let list_for_add = list.clone();
    add.connect_clicked(move |_| {
        open_model_editor(&page_for_add, &list_for_add, None);
    });
    list.pack_start(&add, false, false, 0);
}

fn open_model_editor(page: &gtk::Box, list: &gtk::Box, existing: Option<crate::models::Provider>) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(if existing.is_some() { "Edit model provider" } else { "Add model provider" });
    window.set_default_size(480, 300);
    window.set_modal(true);
    let root = vbox(8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);

    let id = gtk::Entry::new();
    id.set_placeholder_text(Some("Provider id, e.g. aihubmix"));
    let models = gtk::Entry::new();
    models.set_placeholder_text(Some("Model ids, comma separated"));
    let key = gtk::Entry::new();
    key.set_visibility(false);
    key.set_placeholder_text(Some("API key (write-only; blank keeps saved key)"));
    if let Some(provider) = &existing {
        id.set_text(&provider.id);
        id.set_sensitive(false);
        models.set_text(&provider.models.join(", "));
        if crate::models::has_key(&provider.id) {
            key.set_placeholder_text(Some("API key saved; leave blank to keep it"));
        }
    }
    for (label, entry) in [("Provider", &id), ("Models", &models), ("API key", &key)] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let label = text(label, "eb-settings-model-label");
        label.set_size_request(80, -1);
        row.pack_start(&label, false, false, 0);
        row.pack_start(entry, true, true, 0);
        root.pack_start(&row, false, false, 0);
    }
    let error = text("", "eb-settings-model-error");
    root.pack_start(&error, false, false, 0);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let cancel = flat_button("eb-settings-preview");
    cancel.add(&text("Cancel", "eb-settings-preview-label"));
    let save = flat_button("eb-settings-apply");
    save.add(&text("Save", "eb-settings-apply-label"));
    actions.pack_end(&save, false, false, 0);
    actions.pack_end(&cancel, false, false, 0);
    root.pack_start(&actions, false, false, 0);
    window.add(&root);
    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());

    let page = page.clone();
    let list = list.clone();
    let window_for_save = window.clone();
    save.connect_clicked(move |_| {
        let provider_id = id.text().to_string().trim().to_string();
        let models: Vec<String> = models
            .text()
            .split(',')
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .collect();
        if provider_id.is_empty() || models.is_empty() {
            error.set_text("Provider and at least one model are required");
            return;
        }
        let mut config = crate::models::load();
        let custom = !matches!(provider_id.as_str(), "deepseek" | "zai-coding-cn" | "aihubmix");
        if let Some(existing) = &existing {
            if let Some(row) = config.providers.iter_mut().find(|row| row.id == existing.id) {
                row.models = models.clone();
            }
        } else {
            config.providers.push(crate::models::Provider {
                id: provider_id.clone(),
                models: models.clone(),
                custom,
            });
        }
        config.provider = provider_id.clone();
        config.model = models[0].clone();
        config.api_key = key.text().to_string();
        match crate::models::save(&config) {
            Ok(()) => {
                crate::bridge::stop();
                fill_model_rows(&page, &list);
                window_for_save.close();
            }
            Err(message) => error.set_text(&message),
        }
    });
    window.show_all();
}


fn acp_page() -> gtk::Box {
    let page = vbox(8);
    page.pack_start(&text("ACP agents", "eb-settings-title-small"), false, false, 0);
    page.pack_start(
        &text(
            "Choose the external ACP agent. The next message uses it; the current turn is not interrupted.",
            "eb-settings-description",
        ),
        false,
        false,
        0,
    );
    let selected = crate::agents::selected();
    let mut group: Option<gtk::RadioButton> = None;
    for agent in crate::agents::all() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        class(&row, "eb-settings-row");
        let radio = if let Some(first) = &group {
            gtk::RadioButton::from_widget(first)
        } else {
            let first = gtk::RadioButton::new();
            group = Some(first.clone());
            first
        };
        radio.set_active(agent == selected);
        row.pack_start(&radio, false, false, 0);
        let copy = vbox(2);
        copy.pack_start(&text(agent.label(), "eb-settings-name"), false, false, 0);
        copy.pack_start(&text(agent.description(), "eb-settings-description"), false, false, 0);
        copy.pack_start(&text(&agent.command(), "eb-settings-status"), false, false, 0);
        row.pack_start(&copy, true, true, 0);
        let state = if agent.installed() { "installed" } else { "not found" };
        row.pack_end(&text(state, "eb-settings-status"), false, false, 0);
        let status = text("", "eb-settings-status");
        let status_for_toggle = status.clone();
        let agent_for_toggle = agent;
        radio.connect_toggled(move |radio| {
            if !radio.is_active() {
                return;
            }
            match crate::agents::save(agent_for_toggle).and_then(|_| crate::router::set_acp(agent_for_toggle)) {
                Ok(()) => status_for_toggle.set_text("Default set"),
                Err(error) => status_for_toggle.set_text(&format!("Save failed: {error}")),
            }
        });
        row.pack_start(&status, false, false, 0);
        page.pack_start(&row, false, false, 0);
    }
    page
}

fn custom_model_page() -> gtk::Box {
    let page = vbox(8);
    let config = crate::router::current().custom;
    page.pack_start(&text("Custom Model", "eb-settings-title-small"), false, false, 0);
    page.pack_start(
        &text(
            "An OpenAI-compatible endpoint. Requests go through curl against {base}/v1/chat/completions.",
            "eb-settings-description",
        ),
        false,
        false,
        0,
    );
    let name = gtk::Entry::new();
    name.set_text(&config.name);
    name.set_placeholder_text(Some("Display name"));
    let base_url = gtk::Entry::new();
    base_url.set_text(&config.base_url);
    base_url.set_placeholder_text(Some("Base URL, e.g. https://api.example.com/v1"));
    let model = gtk::Entry::new();
    model.set_text(&config.model);
    model.set_placeholder_text(Some("Model id"));
    let key = gtk::Entry::new();
    key.set_visibility(false);
    key.set_placeholder_text(Some(if config.api_key.is_empty() {
        "API key (write-only)"
    } else {
        "API key saved; leave blank to keep it"
    }));
    for (label, entry) in [
        ("Name", &name),
        ("Base URL", &base_url),
        ("Model", &model),
        ("API key", &key),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let label = text(label, "eb-settings-model-label");
        label.set_size_request(80, -1);
        row.pack_start(&label, false, false, 0);
        row.pack_start(entry, true, true, 0);
        page.pack_start(&row, false, false, 0);
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let save = flat_button("eb-settings-apply");
    save.add(&text("Save & Use", "eb-settings-apply-label"));
    let status = text("", "eb-settings-status");
    actions.pack_start(&save, false, false, 0);
    actions.pack_start(&status, true, true, 0);
    page.pack_start(&actions, false, false, 0);
    save.connect_clicked(move |_| {
        let custom = crate::custom_model::CustomConfig {
            name: name.text().to_string(),
            base_url: base_url.text().to_string(),
            api_key: key.text().to_string(),
            model: model.text().to_string(),
        };
        match crate::router::set_custom(custom) {
            Ok(()) => status.set_text("Saved. The next message uses this custom model."),
            Err(error) => status.set_text(&format!("Save failed: {error}")),
        }
    });
    page
}

fn models_page() -> gtk::Box {
    let page = vbox(8);
    let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    let body = vbox(14);
    class(&body, "eb-settings-groups");

    let default_group = vbox(6);
    class(&default_group, "eb-settings-group");
    default_group.pack_start(&text("Default route", "eb-settings-title-small"), false, false, 0);
    default_group.pack_start(
        &text(
            "Choose which answer source new messages use. dsh and custom model are independent of the ACP agent below.",
            "eb-settings-description",
        ),
        false,
        false,
        0,
    );
    let current_label = text(&format!("Current: {}", crate::router::label()), "eb-settings-default-value");
    default_group.pack_start(&current_label, false, false, 0);
    let route_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for (route, label) in [
        (crate::router::DefaultRoute::Acp, "ACP agent"),
        (crate::router::DefaultRoute::Dsh, "dsh"),
        (crate::router::DefaultRoute::Custom, "Custom Model"),
    ] {
        let button = gtk::Button::with_label(label);
        let active = crate::router::current().default == route;
        class(&button, if active { "eb-chip-active" } else { "eb-chip" });
        let current_label = current_label.clone();
        button.connect_clicked(move |_| match crate::router::set_default(route) {
            Ok(()) => current_label.set_text(&format!("Current: {}", crate::router::label())),
            Err(error) => current_label.set_text(&format!("Default failed: {error}")),
        });
        route_row.pack_start(&button, false, false, 0);
    }
    default_group.pack_start(&route_row, false, false, 0);
    body.pack_start(&default_group, false, false, 0);

    let acp = acp_page();
    class(&acp, "eb-settings-group");
    body.pack_start(&acp, false, false, 0);

    let dsh = model_section();
    class(&dsh, "eb-settings-group");
    body.pack_start(&dsh, false, false, 0);

    let custom = custom_model_page();
    class(&custom, "eb-settings-group");
    body.pack_start(&custom, false, false, 0);

    scroller.add(&body);
    page.pack_start(&scroller, true, true, 0);

    // Keep the default label fresh even when a provider/ACP row changes it.
    let current_label_for_timer = current_label.clone();
    gtk::glib::timeout_add_seconds_local(1, move || {
        current_label_for_timer.set_text(&format!("Current: {}", crate::router::label()));
        gtk::glib::ControlFlow::Continue
    });
    page
}

fn plugins_page(controller: Rc<Controller>) -> gtk::Box {
    let page = vbox(8);
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
        check.connect_toggled(move |check| controller_for_toggle.toggle(id, check.is_active()));
        check.set_active(auto_select.split(',').any(|value| value.trim() == plugin.id));
        row.pack_start(&check, false, false, 0);
        let copy = vbox(2);
        copy.pack_start(&text(plugin.title, "eb-settings-name"), false, false, 0);
        copy.pack_start(&text(plugin.description, "eb-settings-description"), false, false, 0);
        row.pack_start(&copy, true, true, 0);
        let status = text("not loaded", "eb-settings-status");
        row.pack_end(&status, false, false, 0);
        status_labels.push((plugin.id, status));
        rows.pack_start(&row, false, false, 0);
    }
    page.pack_start(&rows, false, false, 0);
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
    page.pack_start(&actions, false, false, 0);
    let log_view = text("", "eb-settings-log");
    log_view.set_xalign(0.0);
    log_view.set_selectable(true);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(150);
    scroll.add(&log_view);
    page.pack_start(&scroll, true, true, 0);

    {
        let controller = controller.clone();
        let status_labels = status_labels.clone();
        let log_view = log_view.clone();
        let files = files.clone();
        apply.connect_clicked(move |_| match controller.apply() {
            Ok(()) => {
                for (id, label) in &status_labels {
                    label.set_text(if controller.selected.borrow().contains(id) {
                        "starting…"
                    } else {
                        "disabled"
                    });
                }
                files.set_visible(controller.selected.borrow().contains("fs"));
            }
            Err(error) => log_view.set_text(&format!("error: {error}")),
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
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || files.clicked());
    }
    let controller_for_timer = controller.clone();
    let log_for_timer = log_view.clone();
    let labels_for_timer = status_labels.clone();
    let files_for_timer = files.clone();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let lines = controller_for_timer.drain_log();
        if !lines.is_empty() {
            let mut text = log_for_timer.text().to_string();
            for line in lines {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&line);
                for (id, label) in &labels_for_timer {
                    if let Some(rest) = line.strip_prefix(&format!("{id}: ")) {
                        label.set_text(rest);
                    }
                }
            }
            log_for_timer.set_text(&text);
        }
        files_for_timer.set_visible(controller_for_timer.fs_running.get());
        gtk::glib::ControlFlow::Continue
    });
    page
}

fn simple_page(_title: &str, body: &str) -> gtk::Box {
    let page = vbox(8);
    let label = text(body, "eb-settings-description");
    label.set_line_wrap(true);
    label.set_xalign(0.0);
    page.pack_start(&label, false, false, 0);
    page
}

/// Open the Settings window. The window owns the controller; closing it stops
/// the sidecar so the experiment leaves no process behind.
pub fn open() {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Settings");
    window.set_default_size(1040, 760);
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    class(&root, "eb-settings");
    window.add(&root);

    let sidebar = vbox(4);
    sidebar.set_size_request(240, -1);
    class(&sidebar, "eb-settings-sidebar");
    let brand = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    brand.pack_start(&text("Settings", "eb-settings-title"), false, false, 0);
    let close = gtk::Button::with_label("×");
    class(&close, "eb-skill-detail-close");
    let window_for_close = window.clone();
    close.connect_clicked(move |_| window_for_close.close());
    brand.pack_end(&close, false, false, 0);
    sidebar.pack_start(&brand, false, false, 0);
    let search = gtk::Entry::new();
    search.set_placeholder_text(Some("Search settings"));
    sidebar.pack_start(&search, false, false, 0);

    let content = vbox(6);
    class(&content, "eb-settings-content");
    let page_title = text("General", "eb-settings-page-title");
    let page_subtitle = text("", "eb-settings-subtitle");
    content.pack_start(&page_title, false, false, 0);
    content.pack_start(&page_subtitle, false, false, 0);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_vexpand(true);

    let controller = Controller::new();
    stack.add_named(
        &simple_page(
            "General",
            "EvoBuddy is a GTK host for external ACP agents and the dsh runtime. Use Models to choose an ACP agent or a custom model provider.",
        ),
        "general",
    );
    stack.add_named(&models_page(), "models");
    stack.add_named(&plugins_page(controller.clone()), "plugins");
    stack.add_named(
        &simple_page(
            "Agent presets",
            "Experts and skills live in the main window under Experts · Skills · Connectors. Active expert and enabled skills are applied to the next prompt.",
        ),
        "presets",
    );
    stack.add_named(
        &simple_page("About", "EvoBuddy 0.1.0 — WorkBuddy-style GTK client."),
        "about",
    );
    content.pack_start(&stack, true, true, 0);
    root.pack_start(&sidebar, false, false, 0);
    root.pack_start(&content, true, true, 0);

    let entries = [
        ("general", "General", "Settings", "Overview"),
        ("models", "Models", "Models", "ACP agents or custom dsh providers"),
        ("plugins", "Built-in plugins", "Built-in plugins", "Runtime dsh host plugins"),
        ("presets", "Agent presets", "Agent presets", "Experts and skills"),
        ("about", "About", "About", "Application information"),
    ];
    let nav_buttons: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
    for (index, (page, label, title, subtitle)) in entries.iter().enumerate() {
        let button = gtk::Button::with_label(label);
        class(&button, if index == 0 { "eb-settings-nav-active" } else { "eb-settings-nav" });
        let stack = stack.clone();
        let page = page.to_string();
        let title = title.to_string();
        let subtitle = subtitle.to_string();
        let title_label = page_title.clone();
        let subtitle_label = page_subtitle.clone();
        let buttons = nav_buttons.clone();
        button.connect_clicked(move |_| {
            stack.set_visible_child_name(&page);
            title_label.set_text(&title);
            subtitle_label.set_text(&subtitle);
            for button in buttons.borrow().iter() {
                class(button, "eb-settings-nav");
            }
            for button in buttons.borrow().iter() {
                if button.label().as_deref() == Some(&title) {
                    class(button, "eb-settings-nav-active");
                }
            }
        });
        sidebar.pack_start(&button, false, false, 0);
        nav_buttons.borrow_mut().push(button);
    }
    let search_for_filter = search.clone();
    let buttons_for_filter = nav_buttons.clone();
    search.connect_changed(move |entry| {
        let needle = entry.text().to_ascii_lowercase();
        for button in buttons_for_filter.borrow().iter() {
            let label = button.label().unwrap_or_default().to_ascii_lowercase();
            button.set_visible(needle.is_empty() || label.contains(&needle));
        }
    });

    let alive = Rc::new(Cell::new(true));
    {
        let alive = alive.clone();
        let controller = controller.clone();
        window.connect_destroy(move |_| {
            alive.set(false);
            controller.stop();
        });
    }
    let _ = alive;
    window.show_all();
    if let Ok(path) = std::env::var("EB_SETTINGS_SHOT") {
        let window = window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
            capture_window(&window, &path);
        });
    }
}
