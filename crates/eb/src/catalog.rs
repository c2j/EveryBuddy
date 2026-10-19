//! Agent-facing skills and connectors, modelled on WorkBuddy's skill cache.

use std::path::PathBuf;

#[derive(Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: String,
    pub enabled: bool,
}

const BUILTIN: &[(&str, &str, &str)] = &[
    (
        "code-review",
        "Review a change for correctness, regressions, and missing tests.",
        "When reviewing code, inspect the diff, identify correctness and security risks, \
         call out missing tests, and separate blocking issues from suggestions.",
    ),
    (
        "release-notes",
        "Turn merged changes into concise user-facing release notes.",
        "When writing release notes, group changes by user impact, omit internal refactors, \
         and state migration or breaking changes explicitly.",
    ),
];

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn skills_root() -> PathBuf {
    std::env::var("EB_SKILLS").map(PathBuf::from).unwrap_or_else(|_| home().join(".evobuddy/skills"))
}

fn state_path() -> PathBuf {
    home().join(".evobuddy/skills.json")
}

fn enabled_names() -> Vec<String> {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
}

pub fn skills() -> Vec<Skill> {
    let enabled = enabled_names();
    let mut out = Vec::new();
    for (name, description, body) in BUILTIN {
        out.push(Skill {
            name: (*name).into(),
            description: (*description).into(),
            body: (*body).into(),
            source: "builtin".into(),
            enabled: enabled.iter().any(|item| item == name),
        });
    }
    let root = skills_root();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let dir = entry.path();
            let file = dir.join("SKILL.md");
            let Ok(body) = std::fs::read_to_string(&file) else { continue };
            let name = dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "skill".into());
            if out.iter().any(|skill| skill.name == name) {
                continue;
            }
            let description = body
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty() && !line.starts_with('#'))
                .unwrap_or("")
                .to_string();
            out.push(Skill {
                name: name.clone(),
                description,
                body,
                source: "user".into(),
                enabled: enabled.iter().any(|item| item == &name),
            });
        }
    }
    out
}

pub fn toggle(name: &str) {
    let mut enabled = enabled_names();
    if let Some(index) = enabled.iter().position(|item| item == name) {
        enabled.remove(index);
    } else {
        enabled.push(name.to_string());
    }
    if let Some(parent) = state_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        state_path(),
        serde_json::to_string_pretty(&enabled).unwrap_or_default(),
    );
}

/// Prefix a user prompt with the enabled skill instructions.
pub fn prompt_for(user: &str) -> String {
    let active: Vec<_> = skills().into_iter().filter(|skill| skill.enabled).collect();
    if active.is_empty() {
        return user.to_string();
    }
    let mut prompt = String::from("Use the following active EvoBuddy skills when relevant:\n");
    for skill in active {
        prompt.push_str(&format!("\n## {}\n{}\n", skill.name, skill.body));
    }
    prompt.push_str("\nUser request:\n");
    prompt.push_str(user);
    prompt
}

pub fn connectors() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let root = home().join(".evobuddy/connectors");
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let name = path
                .file_stem()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push((name, "configured".into()));
        }
    }
    if out.is_empty() {
        out.push(("local files".into(), "builtin workspace access".into()));
        out.push(("shell".into(), "agent exec via ACP".into()));
    }
    out
}

fn scheduled_path() -> PathBuf {
    home().join(".evobuddy/scheduled.json")
}

fn scheduled_values() -> Vec<serde_json::Value> {
    std::fs::read_to_string(scheduled_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<serde_json::Value>>(&text).ok())
        .unwrap_or_default()
}

pub fn scheduled() -> Vec<(String, String, String)> {
    scheduled_values()
        .into_iter()
        .map(|value| {
            (
                value["title"].as_str().unwrap_or("Scheduled task").into(),
                value["prompt"].as_str().unwrap_or("").into(),
                value["due_at"].as_str().unwrap_or("manual").into(),
            )
        })
        .collect()
}

/// Claim every job whose numeric `due_at` epoch has passed, removing it from
/// the persisted queue.
pub fn take_due() -> Vec<(String, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0);
    let mut due = Vec::new();
    let mut remaining = Vec::new();
    for value in scheduled_values() {
        let at = value["due_at"]
            .as_u64()
            .or_else(|| value["due_at"].as_str().and_then(|text| text.parse().ok()));
        if at.is_some_and(|at| at <= now) {
            due.push((
                value["title"].as_str().unwrap_or("Scheduled task").into(),
                value["prompt"].as_str().unwrap_or("").into(),
            ));
        } else {
            remaining.push(value);
        }
    }
    if !due.is_empty() {
        let _ = std::fs::write(
            scheduled_path(),
            serde_json::to_string_pretty(&remaining).unwrap_or_default(),
        );
    }
    due
}

