//! Dialogs and state transitions for the sidebar's Workspace actions.

use gtk::prelude::*;

use crate::model::{Store, WorkspaceState};
use crate::sidebar::WorkspaceAction;
use crate::shell::Refresher;

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
