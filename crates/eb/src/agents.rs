//! ACP agent selection for the external-agent path.
//!
//! The host mounts one answer source at startup, so changing this setting takes
//! effect on the next app start. `EVOBUDDY_ACP_AGENT` still wins for ad-hoc runs.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    pub fn program(self) -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        match self {
            Self::Opencode => home.join(".opencode/bin/opencode"),
            Self::Grok => home.join(".grok/bin/grok"),
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
