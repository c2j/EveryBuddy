//! ACP agent selection for the external-agent path.
//!
//! The host mounts one answer source at startup, so changing this setting takes
//! effect on the next app start. `EVOBUDDY_ACP_AGENT` still wins for ad-hoc runs.
//!
//! Three of the four slots are agents that speak ACP themselves. `pi` is not one
//! of them: it only has its own line-delimited `--mode rpc`, so its slot runs the
//! community `pi-acp` adapter, which translates ACP on stdio into that protocol.
//! The distinction is visible in Settings because the adapter, not pi, is what
//! gets spawned.

use std::path::PathBuf;

/// Pinned so the `npx` fallback is reproducible; an installed `pi-acp` wins.
const PI_ADAPTER_VERSION: &str = "0.0.34";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AcpAgent {
    Opencode,
    Grok,
    Jcode,
    Pi,
}

impl AcpAgent {
    pub fn id(self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::Grok => "grok",
            Self::Jcode => "jcode",
            Self::Pi => "pi",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::Grok => "Grok Build",
            Self::Jcode => "jcode",
            Self::Pi => "pi",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Opencode => "opencode acp — local coding agent",
            Self::Grok => "Grok Build — agent stdio",
            Self::Jcode => "jcode acp — native ACP on the jcode daemon",
            Self::Pi => "pi through the pi-acp adapter (pi has no ACP entry point)",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "opencode" => Some(Self::Opencode),
            "grok" => Some(Self::Grok),
            "jcode" => Some(Self::Jcode),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }

    /// Program plus arguments to spawn. `EVOBUDDY_OPENCODE_BIN`,
    /// `EVOBUDDY_GROK_BIN`, `EVOBUDDY_JCODE_BIN` and `EVOBUDDY_PI_ACP_BIN`
    /// override the default location; a bare command name is resolved on PATH.
    pub fn launch(self) -> (PathBuf, Vec<String>) {
        if self == Self::Pi {
            return pi_adapter();
        }
        (self.program(), self.args().iter().map(|arg| arg.to_string()).collect())
    }

    /// The command line as the settings page shows it.
    pub fn command(self) -> String {
        let (program, args) = self.launch();
        format!("{} {}", program.display(), args.join(" "))
    }

    pub fn installed(self) -> bool {
        let (program, _) = self.launch();
        // The adapter is only a translator: without `pi` on PATH the slot would
        // spawn something that fails on the first prompt.
        program.is_file() && (self != Self::Pi || resolve_command(PathBuf::from("pi")).is_file())
    }

    fn program(self) -> PathBuf {
        let env_key = match self {
            Self::Opencode => "EVOBUDDY_OPENCODE_BIN",
            Self::Grok => "EVOBUDDY_GROK_BIN",
            Self::Jcode => "EVOBUDDY_JCODE_BIN",
            Self::Pi => "EVOBUDDY_PI_ACP_BIN",
        };
        if let Some(value) = std::env::var_os(env_key) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() {
                return resolve_command(path);
            }
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let default = match self {
            Self::Opencode => home.join(".opencode/bin/opencode"),
            Self::Grok => home.join(".grok/bin/grok"),
            Self::Jcode => home.join(".local/bin/jcode"),
            // Handled by `pi_adapter`; only reachable through the override above.
            Self::Pi => PathBuf::from("pi-acp"),
        };
        if default.is_file() {
            default
        } else {
            resolve_command(PathBuf::from(self.id()))
        }
    }

    fn args(self) -> Vec<&'static str> {
        match self {
            Self::Opencode => vec!["acp"],
            Self::Grok => vec!["agent", "stdio"],
            Self::Jcode => vec!["acp"],
            Self::Pi => vec![],
        }
    }
}

/// pi's ACP entry point: an installed `pi-acp`, else the pinned adapter through
/// `npx`. Nothing is downloaded until the slot is actually started, so a user who
/// never selects pi never runs third-party code.
fn pi_adapter() -> (PathBuf, Vec<String>) {
    let installed = resolve_command(PathBuf::from("pi-acp"));
    if installed != PathBuf::from("pi-acp") {
        return (installed, vec![]);
    }
    (
        resolve_command(PathBuf::from("npx")),
        vec!["--yes".into(), format!("pi-acp@{PI_ADAPTER_VERSION}")],
    )
}

/// Environment overrides for a spawned external agent. The AppImage puts its
/// own libraries first in `LD_LIBRARY_PATH`; a host-installed opencode/Grok
/// must not inherit them. The launcher records the host value before prepending
/// the bundle.
pub fn external_env() -> Vec<(String, String)> {
    match std::env::var("EVOBUDDY_HOST_LD_LIBRARY_PATH") {
        Ok(value) => vec![("LD_LIBRARY_PATH".to_string(), value)],
        Err(_) => Vec::new(),
    }
}

fn resolve_command(path: PathBuf) -> PathBuf {
    if path.is_absolute() || path.components().count() > 1 || path.is_file() {
        return path;
    }
    let name = path.as_os_str().to_string_lossy().to_string();
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    path
}

fn config_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".evobuddy/acp.json")
}

pub fn selected() -> AcpAgent {
    if let Ok(value) = std::env::var("EVOBUDDY_ACP_AGENT") {
        if let Some(agent) = AcpAgent::parse(&value) {
            return agent;
        }
    }
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(agent) = value["agent"].as_str().and_then(AcpAgent::parse) {
                return agent;
            }
        }
    }
    AcpAgent::Opencode
}

pub fn all() -> [AcpAgent; 4] {
    [
        AcpAgent::Opencode,
        AcpAgent::Grok,
        AcpAgent::Jcode,
        AcpAgent::Pi,
    ]
}

pub fn save(agent: AcpAgent) -> Result<(), String> {
    let root = config_path().parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let value = serde_json::json!({ "agent": agent.id() });
    std::fs::write(
        config_path(),
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    )
    .map_err(|error| error.to_string())?;
    std::env::set_var("EVOBUDDY_ACP_AGENT", agent.id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_have_distinct_commands() {
        assert_ne!(AcpAgent::Opencode.id(), AcpAgent::Grok.id());
        assert!(AcpAgent::Opencode.args().contains(&"acp"));
        assert_eq!(AcpAgent::Grok.args(), vec!["agent", "stdio"]);
        // jcode speaks ACP itself; pi does not, so its slot is the adapter.
        assert_eq!(AcpAgent::Jcode.args(), vec!["acp"]);
        assert!(AcpAgent::Pi.args().is_empty());
        assert_eq!(
            all().iter().map(|agent| agent.id()).collect::<Vec<_>>(),
            vec!["opencode", "grok", "jcode", "pi"]
        );
    }

    #[test]
    fn every_id_round_trips_through_parse() {
        for agent in all() {
            assert_eq!(AcpAgent::parse(agent.id()), Some(agent));
            assert!(AcpAgent::parse("nope").is_none());
        }
    }

    #[test]
    fn the_pi_slot_names_the_pinned_adapter() {
        let (_, args) = pi_adapter();
        let pinned = args.iter().any(|arg| arg == &format!("pi-acp@{PI_ADAPTER_VERSION}"));
        // Either an installed `pi-acp` (no args) or the pinned npx fallback.
        assert!(args.is_empty() || pinned);
    }
}
