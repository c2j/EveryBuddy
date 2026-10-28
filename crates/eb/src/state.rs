//! Task and workspace persistence.
//!
//! Tasks reference a workspace only by id. Legacy `tasks.json` files that still
//! carry a `workspace` path are migrated into workspace entities on load.

use std::path::PathBuf;

use crate::model::{Store, Task, Turn, Workspace};

pub struct Loaded {
    pub tasks: Vec<Task>,
    pub workspaces: Vec<Workspace>,
}

fn path() -> PathBuf {
    std::env::var("EB_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".evobuddy/tasks.json")
        })
}

fn workspaces_path() -> PathBuf {
    path().with_file_name("workspaces.json")
}

fn session_timestamp(session_id: &str) -> Option<u64> {
    let raw = session_id.rsplit('-').next()?.parse::<u64>().ok()?;
    Some(if raw > 1_000_000_000_000 { raw / 1_000 } else { raw })
}

pub fn save(store: &Store) {
    let app = store.borrow();
    let tasks: Vec<_> = app
        .tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "title": task.title,
                "session_id": task.session_id,
                "workspace_id": task.workspace_id,
                "agent_route": task.agent_route,
                "last_route": task.last_route,
                "updated_at": task.updated_at,
                "pinned": task.pinned,
                "archived": task.archived,
                "turns": serde_json::to_value(&task.turns).unwrap_or_default(),
            })
        })
        .collect();
    let workspaces = serde_json::to_value(&app.workspaces).unwrap_or_default();
    drop(app);

    let task_path = path();
    if let Some(parent) = task_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        &task_path,
        serde_json::to_string_pretty(&tasks).unwrap_or_default(),
    );
    let _ = std::fs::write(
        workspaces_path(),
        serde_json::to_string_pretty(&workspaces).unwrap_or_default(),
    );
}

pub fn load() -> Loaded {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Loaded {
            tasks: Vec::new(),
            workspaces: Vec::new(),
        };
    };
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        return Loaded {
            tasks: Vec::new(),
            workspaces: Vec::new(),
        };
    };

    let mut workspaces: Vec<Workspace> = std::fs::read_to_string(workspaces_path())
        .ok()
        .and_then(|text| {
            let parsed = serde_json::from_str(&text);
            if let Err(error) = &parsed {
                eprintln!("[evobuddy] workspaces.json parse failed: {error}");
            }
            parsed.ok()
        })
        .unwrap_or_default();

    let mut tasks = Vec::new();
    for value in values {
        let Some(title) = value["title"].as_str().map(str::to_string) else {
            continue;
        };
        let session_id = value["session_id"].as_str().map(str::to_string);
        let mut workspace_id = value["workspace_id"].as_str().map(str::to_string);
        let updated_at = value["updated_at"]
            .as_u64()
            .or_else(|| session_id.as_deref().and_then(session_timestamp));

        // Migrate the old `workspace` path into a first-class entity. One
        // workspace is created per unique root, so shared paths stay shared.
        if workspace_id.is_none() {
            if let Some(root) = value["workspace"].as_str().filter(|path| !path.is_empty()) {
                let existing = workspaces.iter().find(|workspace| workspace.root == root);
                workspace_id = Some(match existing {
                    Some(workspace) => workspace.id.clone(),
                    None => {
                        let name = if title.trim().is_empty() {
                            std::path::Path::new(root)
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_else(|| "Workspace".into())
                        } else {
                            title.clone()
                        };
                        let workspace = crate::workspace::adopt(root, &name, updated_at);
                        let id = workspace.id.clone();
                        workspaces.push(workspace);
                        id
                    }
                });
            }
        }
        if let Some(id) = &workspace_id {
            if !workspaces.iter().any(|workspace| &workspace.id == id) {
                workspace_id = None;
            }
        }

        tasks.push(Task {
            title,
            meta: String::new(),
            turns: serde_json::from_value(value["turns"].clone()).unwrap_or_default(),
            artifact: None,
            artifacts: Vec::new(),
            session_id,
            workspace_id,
            agent_route: value["agent_route"].as_str().map(str::to_string),
            last_route: value["last_route"].as_str().map(str::to_string),
            usage: None,
            started_at: None,
            updated_at,
            pinned: value["pinned"].as_bool().unwrap_or(false),
            archived: value["archived"].as_bool().unwrap_or(false),
        });
    }

    Loaded { tasks, workspaces }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_turns_round_trip() {
        let turns = vec![
            Turn::User { text: "hello".into(), time: "now".into() },
            Turn::Agent {
                name: "EvoBuddy".into(),
                status: Some("Finished".into()),
                blocks: vec![crate::model::Block::Text("world".into())],
                streaming: false,
            },
        ];
        let value = serde_json::to_value(&turns).unwrap();
        let loaded: Vec<Turn> = serde_json::from_value(value).unwrap();
        assert!(matches!(
            &loaded[0],
            Turn::User { text, .. } if text == "hello"
        ));
        assert!(matches!(
            &loaded[1],
            Turn::Agent { blocks, .. }
                if matches!(&blocks[0], crate::model::Block::Text(text) if text == "world")
        ));
    }
}
