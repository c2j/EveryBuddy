//! Minimal Phase 1 task persistence: task title + retained dsh session id.

use std::path::PathBuf;

use crate::model::Store;

fn path() -> PathBuf {
    std::env::var("EB_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".evobuddy/tasks.json")
        })
}

pub fn save(store: &Store) {
    let tasks: Vec<_> = store
        .borrow()
        .tasks
        .iter()
        .map(|task| serde_json::json!({
            "title": task.title,
            "session_id": task.session_id,
            "workspace": task.workspace,
            "agent_route": task.agent_route,
        }))
        .collect();
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(&tasks).unwrap_or_default());
}

pub fn load() -> Vec<(String, Option<String>, Option<String>, Option<String>)> {
    let Ok(text) = std::fs::read_to_string(path()) else { return Vec::new() };
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else { return Vec::new() };
    values
        .into_iter()
        .filter_map(|value| {
            Some((
                value["title"].as_str()?.to_string(),
                value["session_id"].as_str().map(str::to_string),
                value["workspace"].as_str().map(str::to_string),
                value["agent_route"].as_str().map(str::to_string),
            ))
        })
        .collect()
}
