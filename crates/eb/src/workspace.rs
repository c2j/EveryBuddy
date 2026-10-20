//! Per-task agent workspaces, isolated from the user's project directory.

use std::path::PathBuf;

fn root() -> PathBuf {
    std::env::var("EB_WORKSPACES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".evobuddy/workspaces")
        })
}

/// Create a fresh workspace directory for a task and return its path.
pub fn create(title: &str) -> Option<String> {
    let slug: String = title
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "task".into() } else { slug };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or(0);
    let path = root().join(format!("{slug}-{stamp}"));
    std::fs::create_dir_all(&path).ok()?;
    Some(path.to_string_lossy().to_string())
}
