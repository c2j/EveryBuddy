//! Dialogs and state transitions for the sidebar's Workspace actions.

use gtk::prelude::*;

use crate::model::{Store, WorkspaceState};
use crate::sidebar::WorkspaceAction;
use crate::shell::Refresher;

/// WorkBuddy-style "+ New Workspace": ask for a name, create
/// `<default storage path>/<name>`, and select it.
pub fn prompt_new(window: &gtk::ApplicationWindow, store: &Store, refresher: &Refresher) {
    let dialog = gtk::Dialog::with_buttons(
        Some("New Workspace"),
        Some(window),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Create", gtk::ResponseType::Accept),
        ],
    );
    dialog.set_default_response(gtk::ResponseType::Accept);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Workspace name"));
    entry.set_activates_default(true);
    entry.set_margin_top(12);
    entry.set_margin_bottom(12);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    dialog.content_area().pack_start(&entry, false, false, 0);
    dialog.show_all();
    let response = dialog.run();
    let name = entry.text().to_string();
    dialog.close();
    if response != gtk::ResponseType::Accept {
        return;
    }
    let Some(workspace) = crate::workspace::create_named(&name) else {
        return;
    };
    select_new(workspace, store, refresher);
}

/// WorkBuddy-style "Open Local Folder": adopt a user-chosen directory as a
/// kept workspace without touching its contents.
pub fn open_local(window: &gtk::ApplicationWindow, store: &Store, refresher: &Refresher) {
    let chooser = gtk::FileChooserDialog::new(
        Some("Open Local Folder"),
        Some(window),
        gtk::FileChooserAction::SelectFolder,
    );
    chooser.add_button("Cancel", gtk::ResponseType::Cancel);
    chooser.add_button("Open", gtk::ResponseType::Accept);
    chooser.set_select_multiple(false);
    chooser.show_all();
    let response = chooser.run();
    let folder = chooser.filename();
    chooser.close();
    if response != gtk::ResponseType::Accept {
        return;
    }
    let Some(path) = folder else {
        return;
    };
    let Some(workspace) = crate::workspace::create_at(&path) else {
        return;
    };
    select_new(workspace, store, refresher);
}

/// Insert a freshly created workspace, select it for the current task (or the
/// picker itself on the landing page), and persist.
fn select_new(workspace: crate::model::Workspace, store: &Store, refresher: &Refresher) {
    let mut app = store.borrow_mut();
    // One entity per root: adopting an already-open folder reselects it.
    if let Some(position) = app
        .workspaces
        .iter()
        .position(|existing| existing.root == workspace.root)
    {
        app.workspace = position + 2;
    } else {
        let id = workspace.id.clone();
        app.workspaces.push(workspace);
        app.workspace = app.workspaces.len() + 1;
    }
    let selected = app.workspaces[app.workspace.saturating_sub(2)].id.clone();
    if let crate::model::Route::Task(index) = app.route {
        if let Some(task) = app.tasks.get_mut(index) {
            task.workspace_id = Some(selected);
            task.updated_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|time| time.as_secs())
                .ok();
        }
    }
    drop(app);
    crate::state::save(store);
    refresher.request();
}

/// Task row `...` menu actions (WorkBuddy parity subset).
pub fn run_task(
    window: &gtk::ApplicationWindow,
    store: &Store,
    refresher: &Refresher,
    action: crate::sidebar::TaskAction,
    index: usize,
) {
    match action {
        crate::sidebar::TaskAction::Pin => {
            let mut app = store.borrow_mut();
            if let Some(task) = app.tasks.get_mut(index) {
                task.pinned = !task.pinned;
                task.updated_at = Some(now());
            }
            drop(app);
            crate::state::save(store);
            refresher.request();
        }
        crate::sidebar::TaskAction::Rename => rename_task(window, store, refresher, index),
        crate::sidebar::TaskAction::OpenFolder => open_task_folder(store, index),
        crate::sidebar::TaskAction::Delete => delete_task(window, store, refresher, index),
    }
}

fn rename_task(
    window: &gtk::ApplicationWindow,
    store: &Store,
    refresher: &Refresher,
    index: usize,
) {
    let Some(current) = ({
        let app = store.borrow();
        app.tasks.get(index).map(|task| task.title.clone())
    }) else {
        return;
    };
    let dialog = gtk::Dialog::with_buttons(
        Some("Rename task"),
        Some(window),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Save", gtk::ResponseType::Accept),
        ],
    );
    dialog.set_default_response(gtk::ResponseType::Accept);
    let entry = gtk::Entry::new();
    entry.set_text(&current);
    entry.set_activates_default(true);
    entry.set_margin_top(12);
    entry.set_margin_bottom(12);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    dialog.content_area().pack_start(&entry, false, false, 0);
    dialog.show_all();
    let response = dialog.run();
    let title = entry.text().to_string();
    dialog.close();
    if response != gtk::ResponseType::Accept {
        return;
    }
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    let mut app = store.borrow_mut();
    if let Some(task) = app.tasks.get_mut(index) {
        task.title = title.to_string();
        task.updated_at = Some(now());
    }
    drop(app);
    crate::state::save(store);
    refresher.request();
}

fn open_task_folder(store: &Store, index: usize) {
    let root = {
        let app = store.borrow();
        app.tasks.get(index).and_then(|task| {
            let id = task.workspace_id.as_deref()?;
            app.workspaces
                .iter()
                .find(|workspace| workspace.id == id)
                .map(|workspace| workspace.root.clone())
        })
    };
    let Some(root) = root else {
        return;
    };
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&root).spawn();
    #[cfg(not(target_os = "macos"))]
    let _ = std::process::Command::new("xdg-open").arg(&root).spawn();
}

fn delete_task(
    window: &gtk::ApplicationWindow,
    store: &Store,
    refresher: &Refresher,
    index: usize,
) {
    let Some(title) = ({
        let app = store.borrow();
        app.tasks.get(index).map(|task| task.title.clone())
    }) else {
        return;
    };
    let dialog = gtk::MessageDialog::new(
        Some(window),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        gtk::MessageType::Warning,
        gtk::ButtonsType::None,
        &format!("Delete task “{title}”?

Its conversation turns are removed. A workspace left without tasks is cleaned up."),
    );
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Delete", gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    dialog.show_all();
    let response = dialog.run();
    dialog.close();
    if response != gtk::ResponseType::Accept {
        return;
    }
    {
        let mut app = store.borrow_mut();
        if index < app.tasks.len() {
            app.tasks.remove(index);
        }
        match app.route {
            crate::model::Route::Task(open) if open >= app.tasks.len() => {
                app.route = if app.tasks.is_empty() {
                    crate::model::Route::Welcome
                } else {
                    crate::model::Route::Task(app.tasks.len() - 1)
                };
                app.nav = if app.tasks.is_empty() { Some(0) } else { None };
            }
            _ => {}
        }
        let crate::model::App { workspaces, tasks, .. } = &mut *app;
        crate::workspace::cleanup_temporary(workspaces, tasks);
    }
    crate::state::save(store);
    refresher.request();
}

pub fn run(
    window: &gtk::ApplicationWindow,
    store: &Store,
    refresher: &Refresher,
    action: WorkspaceAction,
    id: &str,
) {
    match action {
        WorkspaceAction::NewUnassigned => {
            let mut app = store.borrow_mut();
            app.workspace = 1;
            app.new_task();
            drop(app);
            refresher.request();
        }
        WorkspaceAction::NewTask => {
            let mut app = store.borrow_mut();
            if let Some(position) = app.workspaces.iter().position(|workspace| workspace.id == id) {
                app.workspace = position + 2;
                app.new_task();
            }
            drop(app);
            refresher.request();
        }
        WorkspaceAction::Keep | WorkspaceAction::Rename => {
            rename(window, store, refresher, id, action);
        }
        WorkspaceAction::Delete => {
            delete(window, store, refresher, id);
        }
    }
}

fn rename(
    window: &gtk::ApplicationWindow,
    store: &Store,
    refresher: &Refresher,
    id: &str,
    action: WorkspaceAction,
) {
    let Some((current, temporary)) = ({
        let app = store.borrow();
        app.workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .map(|workspace| (workspace.name.clone(), workspace.state == WorkspaceState::Temporary))
    }) else {
        return;
    };
    let title = if matches!(action, WorkspaceAction::Keep) {
        "Keep workspace"
    } else {
        "Rename workspace"
    };
    let dialog = gtk::Dialog::with_buttons(
        Some(title),
        Some(window),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Save", gtk::ResponseType::Accept),
        ],
    );
    dialog.set_default_response(gtk::ResponseType::Accept);
    let entry = gtk::Entry::new();
    entry.set_text(&current);
    entry.set_activates_default(true);
    entry.set_margin_top(12);
    entry.set_margin_bottom(12);
    entry.set_margin_start(12);
    entry.set_margin_end(12);
    dialog.content_area().pack_start(&entry, false, false, 0);
    dialog.show_all();
    let response = dialog.run();
    let name = entry.text().to_string();
    dialog.close();
    if response != gtk::ResponseType::Accept {
        return;
    }
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let mut app = store.borrow_mut();
    if let Some(workspace) = app
        .workspaces
        .iter_mut()
        .find(|workspace| workspace.id == id)
    {
        workspace.name = name.to_string();
        workspace.updated_at = now();
        // Renaming or keeping an explicit workspace makes it durable.
        if temporary || matches!(action, WorkspaceAction::Keep | WorkspaceAction::Rename) {
            workspace.state = WorkspaceState::Kept;
        }
    }
    drop(app);
    crate::state::save(store);
    refresher.request();
}

fn delete(
    window: &gtk::ApplicationWindow,
    store: &Store,
    refresher: &Refresher,
    id: &str,
) {
    let Some(name) = ({
        let app = store.borrow();
        app.workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .map(crate::workspace::display_name)
    }) else {
        return;
    };
    let dialog = gtk::MessageDialog::new(
        Some(window),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        gtk::MessageType::Warning,
        gtk::ButtonsType::None,
        &format!(
            "Delete workspace “{name}”?\n\nIts tasks stay, but become unassigned. The workspace directory is removed."
        ),
    );
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Delete", gtk::ResponseType::Accept);
    dialog.set_default_response(gtk::ResponseType::Cancel);
    dialog.show_all();
    let response = dialog.run();
    dialog.close();
    if response != gtk::ResponseType::Accept {
        return;
    }

    let root = {
        let mut app = store.borrow_mut();
        let Some(position) = app.workspaces.iter().position(|workspace| workspace.id == id) else {
            return;
        };
        let workspace = app.workspaces.remove(position);
        for task in &mut app.tasks {
            if task.workspace_id.as_deref() == Some(id) {
                task.workspace_id = None;
            }
        }
        // Keep the picker on Auto after its selected workspace disappears.
        app.workspace = 0;
        workspace.root
    };
    crate::workspace::remove_root(&root);
    crate::state::save(store);
    refresher.request();
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0)
}
