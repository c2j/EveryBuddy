//! First-class Workspaces: named execution environments owned by EvoBuddy.
//!
//! A workspace has a stable id and owns a root directory. Tasks reference the
//! id; they never use the path as an identity or a grouping key.

use std::path::PathBuf;

use crate::model::{Workspace, WorkspaceState};

pub fn legacy_root() -> PathBuf {
    std::env::var("EB_WORKSPACES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".evobuddy/workspaces")
        })
}

fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".evobuddy/workspace-settings.json")
}

/// The user-configurable directory new workspaces are created under
/// (Settings → General). Defaults to `~/EvoBuddy`.
pub fn default_root() -> PathBuf {
    if let Ok(text) = std::fs::read_to_string(settings_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(root) = value["default_root"].as_str() {
                if !root.trim().is_empty() {
                    return PathBuf::from(root);
                }
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("EvoBuddy")
}

pub fn set_default_root(path: &std::path::Path) -> Result<(), String> {
    let path = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    let value = serde_json::json!({
        "default_root": path.to_string_lossy().to_string(),
    });
    if let Some(parent) = settings_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(
        settings_path(),
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    )
    .map_err(|error| error.to_string())
}

fn root() -> PathBuf {
    default_root()
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

/// Create a user-named workspace at `<default_root>/<name>` (WorkBuddy's
/// "+ New Workspace"). Reuses the directory when it already exists, and
/// dedupes against an entity that already points at the same root.
pub fn create_named(name: &str) -> Option<Workspace> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let path = default_root().join(name);
    std::fs::create_dir_all(&path).ok()?;
    let root = path.to_string_lossy().to_string();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0);
    Some(Workspace {
        id: format!("ws-{stamp}-{}", stable_suffix(&root)),
        name: name.to_string(),
        root,
        created_at: stamp,
        updated_at: stamp,
        state: WorkspaceState::Kept,
    })
}

/// Adopt a user-chosen local directory as a kept workspace (WorkBuddy's
/// "Open Local Folder"). The directory's contents are never touched.
pub fn create_at(dir: &std::path::Path) -> Option<Workspace> {
    let path = dir.canonicalize().ok()?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".into());
    let root = path.to_string_lossy().to_string();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0);
    Some(Workspace {
        id: format!("ws-{stamp}-{}", stable_suffix(&root)),
        name,
        root,
        created_at: stamp,
        updated_at: stamp,
        state: WorkspaceState::Kept,
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

/// Remove a workspace root only when EvoBuddy created it: under the current
/// default storage path or the legacy area. User-chosen local folders are
/// kept on disk ("changes do not affect existing data").
pub fn remove_root(path: &str) -> bool {
    let path = std::path::Path::new(path);
    let managed = path.starts_with(root()) || path.starts_with(legacy_root());
    if managed {
        let _ = std::fs::remove_dir_all(path);
    }
    managed
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
