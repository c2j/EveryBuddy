//! ACP agent selection for the external-agent path.
//!
//! The host mounts one answer source at startup, so changing this setting takes
//! effect on the next app start. `EVOBUDDY_ACP_AGENT` still wins for ad-hoc runs.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AcpAgent {
    Opencode,
    Grok,
}

impl AcpAgent {
    pub fn id(self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::Grok => "grok",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::Grok => "Grok Build",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Opencode => "opencode acp — local coding agent",
            Self::Grok => "Grok Build — agent stdio",
        }
    }

    /// The binary to spawn. `EVOBUDDY_OPENCODE_BIN` / `EVOBUDDY_GROK_BIN`
    /// override the default location; a bare command name is resolved on PATH.
    pub fn program(self) -> PathBuf {
        let env_key = match self {
            Self::Opencode => "EVOBUDDY_OPENCODE_BIN",
            Self::Grok => "EVOBUDDY_GROK_BIN",
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
        };
        if default.is_file() {
            default
        } else {
            resolve_command(PathBuf::from(self.id()))
        }
    }

    pub fn args(self) -> Vec<&'static str> {
        match self {
            Self::Opencode => vec!["acp"],
            Self::Grok => vec!["agent", "stdio"],
        }
    }

    pub fn installed(self) -> bool {
        self.program().is_file()
    }

    pub fn command(self) -> String {
        format!("{} {}", self.program().display(), self.args().join(" "))
    }
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
        if value == "grok" {
            return AcpAgent::Grok;
        }
        if value == "opencode" {
            return AcpAgent::Opencode;
        }
    }
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            match value["agent"].as_str() {
                Some("grok") => return AcpAgent::Grok,
                Some("opencode") => return AcpAgent::Opencode,
                _ => {}
            }
        }
    }
    AcpAgent::Opencode
}

pub fn all() -> [AcpAgent; 2] {
    [AcpAgent::Opencode, AcpAgent::Grok]
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
    }
}
