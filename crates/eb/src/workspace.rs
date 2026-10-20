//! First-class Workspaces: named execution environments owned by EvoBuddy.
//!
//! A workspace has a stable id and owns a root directory. Tasks reference the
//! id; they never use the path as an identity or a grouping key.

use std::path::PathBuf;

use crate::model::{Workspace, WorkspaceState};

pub fn root() -> PathBuf {
    std::env::var("EB_WORKSPACES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".evobuddy/workspaces")
        })
}

fn slug(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "workspace".into()
    } else {
        slug
    }
}

/// Create a temporary workspace and its root directory. The caller owns
/// inserting it into `App::workspaces`.
pub fn create(name: &str) -> Option<Workspace> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or(0);
    let slug = slug(name);
    let path = root().join(format!("{slug}-{stamp}"));
    std::fs::create_dir_all(&path).ok()?;
    let seconds = (stamp / 1_000) as u64;
    Some(Workspace {
        id: format!("ws-{stamp}"),
        name: name.trim().to_string(),
        root: path.to_string_lossy().to_string(),
        created_at: seconds,
        updated_at: seconds,
        state: WorkspaceState::Temporary,
    })
}

/// Create a workspace entity for an existing root during migration.
pub fn adopt(root: &str, name: &str, updated_at: Option<u64>) -> Workspace {
    let seconds = updated_at.unwrap_or(0);
    Workspace {
        id: format!("ws-legacy-{seconds}-{}", stable_suffix(root)),
        name: name.trim().to_string(),
        root: root.to_string(),
        created_at: seconds,
        updated_at: seconds,
        state: WorkspaceState::Kept,
    }
}

fn stable_suffix(value: &str) -> u64 {
    // Stable across runs and independent of hash iteration order.
    value
        .bytes()
        .fold(1_469_598_103_934_665_603u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211)
        })
}

pub fn find<'a>(workspaces: &'a [Workspace], id: &str) -> Option<&'a Workspace> {
    workspaces.iter().find(|workspace| workspace.id == id)
}

pub fn find_mut<'a>(
    workspaces: &'a mut [Workspace],
    id: &str,
) -> Option<&'a mut Workspace> {
    workspaces.iter_mut().find(|workspace| workspace.id == id)
}

/// Remove a workspace root only when it lives under EvoBuddy's managed area.
pub fn remove_root(path: &str) {
    let path = std::path::Path::new(path);
    if path.starts_with(root()) {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// Remove Temporary workspaces that no task references. Kept workspaces and
/// workspaces with tasks survive. Returns the number removed.
pub fn cleanup_temporary(
    workspaces: &mut Vec<Workspace>,
    tasks: &[crate::model::Task],
) -> usize {
    let used: std::collections::HashSet<&str> = tasks
        .iter()
        .filter_map(|task| task.workspace_id.as_deref())
        .collect();
    let before = workspaces.len();
    let removed: Vec<Workspace> = workspaces
        .iter()
        .filter(|workspace| {
            workspace.state == WorkspaceState::Temporary && !used.contains(workspace.id.as_str())
        })
        .cloned()
        .collect();
    workspaces.retain(|workspace| {
        !(workspace.state == WorkspaceState::Temporary && !used.contains(workspace.id.as_str()))
    });
    for workspace in removed {
        remove_root(&workspace.root);
    }
    before - workspaces.len()
}

pub fn display_name(workspace: &Workspace) -> String {
    if workspace.name.trim().is_empty() {
        std::path::Path::new(&workspace.root)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| workspace.id.clone())
    } else {
        workspace.name.clone()
    }
}
