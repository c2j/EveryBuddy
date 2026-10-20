//! Task persistence: title, retained dsh session id, route, and the visible
//! conversation turns. The dsh session is the agent's source of truth; the
//! turns are the projection the sidebar needs after a restart.

use std::path::PathBuf;

use crate::model::{Store, Turn};

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
            "last_route": task.last_route,
            "updated_at": task.updated_at,
            "turns": serde_json::to_value(&task.turns).unwrap_or_default(),
        }))
        .collect();
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(&tasks).unwrap_or_default());
}

pub fn load() -> Vec<(
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Vec<Turn>,
    Option<u64>,
)> {
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
                value["last_route"].as_str().map(str::to_string),
                serde_json::from_value(value["turns"].clone()).unwrap_or_default(),
                value["updated_at"].as_u64(),
            ))
        })
        .collect()
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
