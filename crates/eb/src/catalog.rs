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
    (
        "test-plan",
        include_str!("../assets/skills/test-plan/SKILL.md"),
    ),
];

pub struct Expert {
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub tags: &'static [&'static str],
    pub avatar: &'static str,
}

const EXPERTS: &[Expert] = &[
    Expert { name: "Code Review Expert", description: "Examines changes for correctness, regressions, and missing tests.", category: "Engineering", tags: &["Code Review", "Quality Gate"], avatar: "🧑‍💻" },
    Expert { name: "Data Analytics Reporter", description: "Transforms complex data into clear reports and visual explanations.", category: "Data & AI", tags: &["Data Analysis", "Visualization"], avatar: "📊" },
    Expert { name: "UI Designer", description: "Designs component systems, layouts, and interaction details.", category: "Product Design", tags: &["UI Design", "Components"], avatar: "🎨" },
    Expert { name: "Frontend Developer", description: "Builds and reviews accessible desktop and web interfaces.", category: "Engineering", tags: &["Frontend", "Accessibility"], avatar: "🖥️" },
    Expert { name: "Content Creator", description: "Turns source material into structured, publishable content.", category: "Content & Creative", tags: &["Writing", "Content"], avatar: "✍️" },
    Expert { name: "Brand Strategist", description: "Clarifies positioning, voice, and brand consistency.", category: "Marketing & Growth", tags: &["Brand", "Strategy"], avatar: "💡" },
    Expert { name: "Research Analyst", description: "Gathers evidence first and separates observation from inference.", category: "Data & AI", tags: &["Research", "Evidence"], avatar: "🔎" },
    Expert { name: "Project Manager", description: "Breaks work into milestones, owners, and verifiable outcomes.", category: "Operations", tags: &["Planning", "Delivery"], avatar: "📋" },
    Expert { name: "Career Coach", description: "Helps tailor resumes, interviews, and career narratives.", category: "Sales & Business", tags: &["Career", "Interview"], avatar: "🧭" },
];

pub fn expert_cards() -> &'static [Expert] {
    EXPERTS
}

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
    EXPERTS.iter().map(|expert| (expert.name, expert.description)).collect()
}

pub fn active_expert() -> Option<&'static str> {
    let name = std::fs::read_to_string(home().join(".evobuddy/expert")).ok()?;
    EXPERTS
        .iter()
        .find(|expert| expert.name == name.trim())
        .map(|expert| expert.name)
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

/// Create a user skill from the Add Skill dialog. Returns the new SKILL.md path.
pub fn create_skill(name: &str, description: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let slug: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "skill".into() } else { slug };
    let dir = skills_root().join(&slug);
    if dir.exists() {
        return Err(format!("skill '{slug}' already exists"));
    }
    std::fs::create_dir_all(&dir).map_err(|error| format!("cannot create skill dir: {error}"))?;
    let file = dir.join("SKILL.md");
    let body = format!(
        "---\nname: {slug}\ndescription: {}\ntriggers: []\n---\nWhen this skill is active, follow these instructions.\n",
        description.trim().replace('\n', " ")
    );
    std::fs::write(&file, body).map_err(|error| format!("cannot write SKILL.md: {error}"))?;
    Ok(file.to_string_lossy().to_string())
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
        if let Some(profile) = EXPERTS.iter().find(|profile| profile.name == expert) {
            prompt.push_str(&format!("Act as the {expert} expert. {}\n\n", profile.description));
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

pub struct ConnectorCard {
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
}

const CONNECTORS: &[ConnectorCard] = &[
    ConnectorCard { name: "Gmail", description: "Search, read, draft, send, and reply to mail.", icon: "✉" },
    ConnectorCard { name: "GitHub", description: "Clone, push, and manage repositories and issues.", icon: "◆" },
    ConnectorCard { name: "Notion", description: "Create, search, and manage Notion pages.", icon: "N" },
    ConnectorCard { name: "Linear", description: "Manage issues, projects, and documents.", icon: "L" },
    ConnectorCard { name: "Feishu", description: "Manage Feishu documents and messages via CLI.", icon: "F" },
    ConnectorCard { name: "WeCom", description: "Official CLI suite for WeChat Work.", icon: "W" },
    ConnectorCard { name: "DingTalk", description: "Manage DingTalk documents and conversations.", icon: "D" },
    ConnectorCard { name: "Tencent Meeting", description: "Create and query Tencent Meeting rooms.", icon: "▣" },
    ConnectorCard { name: "Microsoft 365", description: "Work with Outlook mail, calendar, and files.", icon: "O" },
];

pub fn connector_cards() -> &'static [ConnectorCard] {
    CONNECTORS
}

pub struct ConnectorEntry {
    pub name: String,
    pub transport: String,
    pub path: String,
    pub config: serde_json::Value,
    pub error: Option<String>,
}

impl ConnectorEntry {
    /// Validate the connector file, then perform an MCP initialize handshake.
    /// Stdio is tested by spawning the command; HTTP by POSTing initialize via
    /// curl. Both are bounded and return the observed failure reason.
    pub fn test(&self) -> Result<String, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let mut request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "evobuddy", "version": env!("CARGO_PKG_VERSION") }
            }
        });
        if self.transport == "stdio" {
            use std::io::{BufRead, BufReader, Write};
            let command = self.config["command"].as_str().unwrap_or("");
            let args: Vec<&str> = self.config["args"]
                .as_array()
                .map(|args| args.iter().filter_map(|arg| arg.as_str()).collect())
                .unwrap_or_default();
            let mut child = std::process::Command::new(command)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|error| format!("cannot start '{command}': {error}"))?;
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = writeln!(stdin, "{}", request);
                let _ = stdin.flush();
            }
            let stdout = child.stdout.take();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                if let Some(stdout) = stdout {
                    let mut line = String::new();
                    let _ = BufReader::new(stdout).read_line(&mut line);
                    let _ = tx.send(line);
                }
            });
            let line = rx.recv_timeout(std::time::Duration::from_secs(4)).unwrap_or_default();
            let _ = child.kill();
            if line.contains("\"result\"") || line.contains("serverInfo") {
                Ok("MCP initialize ok".into())
            } else if line.trim().is_empty() {
                Err("no MCP response before timeout".into())
            } else {
                Err(format!("unexpected MCP response: {}", line.trim()))
            }
        } else if self.transport == "http" {
            let url = self.config["url"].as_str().unwrap_or("");
            let output = std::process::Command::new("curl")
                .args([
                    "-sS",
                    "-m",
                    "4",
                    "-X",
                    "POST",
                    url,
                    "-H",
                    "Content-Type: application/json",
                    "-H",
                    "Accept: application/json, text/event-stream",
                    "-d",
                    &request.to_string(),
                ])
                .output()
                .map_err(|error| format!("curl failed: {error}"))?;
            let body = String::from_utf8_lossy(&output.stdout);
            if body.contains("\"result\"") || body.contains("serverInfo") {
                Ok("MCP initialize ok".into())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("no MCP result ({})", stderr.trim()))
            }
        } else {
            Err(format!("unknown transport '{}'", self.transport))
        }
    }
}

fn validate_connector(name: &str, config: &serde_json::Value, path: &str) -> ConnectorEntry {
    let mut error = None;
    let command = config["command"].as_str().unwrap_or("");
    let url = config["url"].as_str().unwrap_or("");
    if command.is_empty() && url.is_empty() {
        error = Some("missing command or url".into());
    }
    if !command.is_empty() {
        if !config["args"].is_null() && !config["args"].is_array() {
            error = Some("args must be an array".into());
        }
        if !config["env"].is_null() && !config["env"].is_object() {
            error = Some("env must be an object".into());
        }
    }
    if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        error = Some("url must be http(s)".into());
    }
    ConnectorEntry {
        name: name.into(),
        transport: transport(config),
        path: path.into(),
        config: config.clone(),
        error,
    }
}

pub fn configured_connectors() -> Vec<ConnectorEntry> {
    let mut out = Vec::new();
    let root = home().join(".evobuddy/connectors");
    let Ok(entries) = std::fs::read_dir(root) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let path_text = path.to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            out.push(ConnectorEntry {
                name: path.file_stem().map(|name| name.to_string_lossy().to_string()).unwrap_or_default(),
                transport: "unknown".into(),
                path: path_text,
                config: serde_json::Value::Null,
                error: Some("cannot read connector file".into()),
            });
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            out.push(ConnectorEntry {
                name: path.file_stem().map(|name| name.to_string_lossy().to_string()).unwrap_or_default(),
                transport: "unknown".into(),
                path: path_text,
                config: serde_json::Value::Null,
                error: Some("invalid JSON".into()),
            });
            continue;
        };
        if let Some(group) = value["mcpServers"].as_object() {
            for (name, config) in group {
                out.push(validate_connector(name, config, &path_text));
            }
        } else {
            let name = value["name"].as_str().unwrap_or("connector");
            out.push(validate_connector(name, &value, &path_text));
        }
    }
    out
}

pub fn create_connector(name: &str, command: &str, args: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || command.trim().is_empty() {
        return Err("name and command are required".into());
    }
    let slug: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let root = home().join(".evobuddy/connectors");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let path = root.join(format!("{slug}.json"));
    if path.exists() {
        return Err(format!("connector '{slug}' already exists"));
    }
    let args: Vec<&str> = args.split_whitespace().collect();
    let value = serde_json::json!({
        "name": name,
        "command": command.trim(),
        "args": args,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap_or_default())
        .map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
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
    let mut out: Vec<(String, String)> = configured_connectors()
        .into_iter()
        .map(|connector| {
            let state = connector
                .error
                .clone()
                .unwrap_or_else(|| connector.transport.clone());
            (connector.name, state)
        })
        .collect();
    if out.is_empty() {
        out.push(("No connectors configured".into(), "add ~/.evobuddy/connectors/*.json".into()));
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

    #[test]
    fn connector_validation_rejects_bad_transport_and_args() {
        let bad = validate_connector("bad", &serde_json::json!({}), "x");
        assert!(bad.error.is_some());
        let bad_args = validate_connector(
            "bad",
            &serde_json::json!({"command":"echo","args":"nope"}),
            "x",
        );
        assert!(bad_args.error.is_some());
    }

    #[test]
    fn connector_stdio_initialize_handshake_is_bounded() {
        let script = "import sys,json\nline=sys.stdin.readline()\nprint(json.dumps({'jsonrpc':'2.0','id':1,'result':{'protocolVersion':'2024-11-05','serverInfo':{'name':'fixture'}}}),flush=True)";
        let entry = validate_connector(
            "fixture",
            &serde_json::json!({"command":"python3","args":["-c",script]}),
            "fixture",
        );
        assert!(entry.error.is_none());
        assert_eq!(entry.test().unwrap(), "MCP initialize ok");
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
