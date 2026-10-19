//! Agent-facing skills, experts, connectors, and the persisted schedule queue.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub body: String,
    pub source: String,
    pub path: String,
    pub enabled: bool,
    pub error: Option<String>,
}

const BUILTIN: &[(&str, &str)] = &[
    (
        "code-review",
        include_str!("../assets/skills/code-review/SKILL.md"),
    ),
    (
        "release-notes",
        include_str!("../assets/skills/release-notes/SKILL.md"),
    ),
];

const EXPERTS: &[(&str, &str)] = &[
    ("Code reviewer", "Prioritise correctness, regressions, and missing tests."),
    ("Researcher", "Gather evidence first, distinguish observation from inference."),
    ("Writer", "Produce concise, structured prose with explicit assumptions."),
];

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn skills_root() -> PathBuf {
    std::env::var("EB_SKILLS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home().join(".evobuddy/skills"))
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

pub fn experts() -> Vec<(&'static str, &'static str)> {
    EXPERTS.to_vec()
}

pub fn active_expert() -> Option<&'static str> {
    let name = std::fs::read_to_string(home().join(".evobuddy/expert")).ok()?;
    EXPERTS
        .iter()
        .find(|(expert, _)| *expert == name.trim())
        .map(|(expert, _)| *expert)
}

pub fn set_expert(name: Option<&str>) {
    let path = home().join(".evobuddy/expert");
    match name {
        Some(name) => {
            let _ = std::fs::write(path, name);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Parse a `SKILL.md` with optional `---` frontmatter. The parser deliberately
/// accepts only flat key/value metadata; malformed frontmatter becomes a visible
/// error instead of silently dropping the skill.
fn parse_skill(raw: &str, fallback_name: &str, source: &str, path: &str) -> Skill {
    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut triggers = Vec::new();
    let mut error = None;
    let mut body = raw.trim().to_string();
    if let Some(rest) = raw.strip_prefix("---") {
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            body = rest[end + 4..].trim().to_string();
            for line in front.lines() {
                let Some((key, value)) = line.split_once(':') else { continue };
                let value = value.trim().trim_matches('"').trim_matches('\'');
                match key.trim() {
                    "name" if !value.is_empty() => name = value.to_string(),
                    "description" => description = value.to_string(),
                    "triggers" => {
                        triggers = value
                            .trim_matches(['[', ']'])
                            .split(',')
                            .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
                            .filter(|item| !item.is_empty())
                            .collect();
                    }
                    _ => {}
                }
            }
        } else {
            error = Some("frontmatter has no closing ---".into());
        }
    }
    if description.is_empty() {
        description = body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .unwrap_or("")
            .to_string();
    }
    if body.is_empty() {
        error = Some("skill body is empty".into());
    }
    Skill {
        name,
        description,
        triggers,
        body,
        source: source.into(),
        path: path.into(),
        enabled: false,
        error,
    }
}

pub fn skills() -> Vec<Skill> {
    let enabled = enabled_names();
    let mut out = Vec::new();
    for (name, raw) in BUILTIN {
        let path = format!("builtin:{name}");
        let mut skill = parse_skill(raw, name, "builtin", &path);
        skill.enabled = enabled.iter().any(|item| item == &skill.name);
        out.push(skill);
    }
    let root = skills_root();
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut dirs: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        dirs.sort();
        for dir in dirs {
            let file = dir.join("SKILL.md");
            let fallback = dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "skill".into());
            let path = file.to_string_lossy().to_string();
            let raw = match std::fs::read_to_string(&file) {
                Ok(raw) => raw,
                Err(error) => {
                    out.push(Skill {
                        name: fallback,
                        description: String::new(),
                        triggers: Vec::new(),
                        body: String::new(),
                        source: "user".into(),
                        path,
                        enabled: false,
                        error: Some(format!("cannot read SKILL.md: {error}")),
                    });
                    continue;
                }
            };
            let mut skill = parse_skill(&raw, &fallback, "user", &path);
            if out.iter().any(|existing| existing.name == skill.name) {
                skill.error = Some(format!("duplicate skill name '{}'", skill.name));
            }
            skill.enabled = enabled.iter().any(|item| item == &skill.name);
            out.push(skill);
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

fn compose_prompt(user: &str, expert: Option<&str>, skills: &[Skill]) -> String {
    let mut prompt = String::new();
    if let Some(expert) = expert {
        if let Some((_, description)) = EXPERTS.iter().find(|(name, _)| *name == expert) {
            prompt.push_str(&format!("Act as the {expert} expert. {description}\n\n"));
        }
    }
    let active: Vec<_> = skills
        .iter()
        .filter(|skill| skill.enabled && skill.error.is_none() && !skill.body.is_empty())
        .collect();
    if !active.is_empty() {
        prompt.push_str("Use these active EvoBuddy skills when relevant:\n");
        let mut budget = 12_000usize;
        for skill in active {
            let mut body = skill.body.as_str();
            if body.len() > budget {
                body = &body[..body.floor_char_boundary(budget)];
            }
            budget = budget.saturating_sub(body.len());
            prompt.push_str(&format!("\n### {}\n{}\n", skill.name, body));
            if budget == 0 {
                prompt.push_str("\n[additional skill text omitted by the prompt budget]\n");
                break;
            }
        }
        prompt.push('\n');
    }
    if prompt.is_empty() {
        return user.to_string();
    }
    prompt.push_str("User request:\n");
    prompt.push_str(user);
    prompt
}

pub fn prompt_for(user: &str) -> String {
    compose_prompt(user, active_expert(), &skills())
}

fn transport(config: &serde_json::Value) -> String {
    if config["command"].as_str().is_some() {
        "stdio".into()
    } else if config["url"].as_str().is_some() {
        "http".into()
    } else {
        "configured".into()
    }
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
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let stem = path
                .file_stem()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "connector".into());
            if let Some(group) = value["mcpServers"].as_object() {
                for (name, config) in group {
                    out.push((name.clone(), transport(config)));
                }
            } else {
                let name = value["name"].as_str().unwrap_or(&stem).to_string();
                out.push((name, transport(&value)));
            }
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
                value["due_at"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| value["due_at"].as_u64().map(|at| at.to_string()))
                    .unwrap_or_else(|| "manual".into()),
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

pub fn add_scheduled(title: &str, prompt: &str, delay_secs: u64) {
    let mut values = scheduled_values();
    let due = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0)
        + delay_secs;
    values.push(serde_json::json!({
        "title": title,
        "prompt": prompt,
        "due_at": due,
    }));
    if let Some(parent) = scheduled_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        scheduled_path(),
        serde_json::to_string_pretty(&values).unwrap_or_default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_triggers() {
        let raw = "---\nname: demo\ndescription: A demo.\ntriggers: [one, two]\n---\nBody text.\n";
        let skill = parse_skill(raw, "fallback", "user", "x");
        assert_eq!(skill.name, "demo");
        assert_eq!(skill.description, "A demo.");
        assert_eq!(skill.triggers, vec!["one", "two"]);
        assert_eq!(skill.body, "Body text.");
        assert!(skill.error.is_none());
    }

    #[test]
    fn malformed_frontmatter_is_visible() {
        let skill = parse_skill("---\nname: broken\nBody", "broken", "user", "x");
        assert!(skill.error.is_some());
    }

    #[test]
    fn enabled_skills_are_injected_and_disabled_ones_are_not() {
        let mut skill = parse_skill("---\nname: demo\n---\nFollow the demo rule.", "demo", "user", "x");
        assert!(!compose_prompt("hello", None, &[skill.clone()]).contains("demo rule"));
        skill.enabled = true;
        let prompt = compose_prompt("hello", None, &[skill]);
        assert!(prompt.contains("### demo"));
        assert!(prompt.ends_with("User request:\nhello"));
    }

    #[test]
    fn no_active_context_returns_the_user_prompt_unchanged() {
        assert_eq!(compose_prompt("hello", None, &[]), "hello");
    }
}

pub fn remove_scheduled(index: usize) {
    let mut values = scheduled_values();
    if index < values.len() {
        values.remove(index);
        let _ = std::fs::write(
            scheduled_path(),
            serde_json::to_string_pretty(&values).unwrap_or_default(),
        );
    }
}

pub fn run_scheduled(index: usize) {
    let mut values = scheduled_values();
    if let Some(value) = values.get_mut(index) {
        value["due_at"] = serde_json::json!(0);
        let _ = std::fs::write(
            scheduled_path(),
            serde_json::to_string_pretty(&values).unwrap_or_default(),
        );
    }
}
