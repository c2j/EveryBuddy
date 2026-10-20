//! The live answer-source router.
//!
//! Host::mount installs this router once. Settings update its route in place;
//! each new turn resolves the current default without rebuilding GTK or the
//! rutis host. Existing turns keep the source they started on.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use eb_api::{AgentSink, AnswerHandle, AnswerSource, PermissionPrompter};
use eb_plugin_acp_agent::{AcpAgent, AcpConfig};

use crate::agents::{self, AcpAgent as AcpChoice};
use crate::custom_model::{CustomConfig, CustomModel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultRoute {
    Acp,
    Dsh,
    Custom,
    Demo,
}

impl DefaultRoute {
    pub fn id(self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Dsh => "dsh",
            Self::Custom => "custom",
            Self::Demo => "demo",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Acp => "ACP agent",
            Self::Dsh => "dsh",
            Self::Custom => "Custom Model",
            Self::Demo => "Demo",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "dsh" => Self::Dsh,
            "custom" => Self::Custom,
            "demo" => Self::Demo,
            _ => Self::Acp,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RouterConfig {
    pub default: DefaultRoute,
    pub acp: AcpChoice,
    pub custom: CustomConfig,
}

impl Default for RouterConfig {
    fn default() -> Self {
        let env_agent = std::env::var("EVOBUDDY_ACP_AGENT").ok();
        let default = if std::env::var("EB_AGENT").ok().as_deref() == Some("demo")
            || crate::selftest::requested()
        {
            DefaultRoute::Demo
        } else if std::env::var("EB_AGENT").ok().as_deref() == Some("dsh") {
            DefaultRoute::Dsh
        } else if env_agent.as_deref() == Some("grok") {
            DefaultRoute::Acp
        } else {
            DefaultRoute::Acp
        };
        Self {
            default,
            acp: if env_agent.as_deref() == Some("grok") {
                AcpChoice::Grok
            } else {
                AcpChoice::Opencode
            },
            custom: CustomConfig::default(),
        }
    }
}

fn config_path() -> PathBuf {
    home().join(".evobuddy/router.json")
}

fn credentials_path() -> PathBuf {
    home().join(".evobuddy/credentials.json")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn load() -> RouterConfig {
    let mut config = RouterConfig::default();
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(default) = value["default"].as_str() {
                config.default = DefaultRoute::parse(default);
            }
            if let Some(acp) = value["acp"].as_str() {
                config.acp = if acp == "grok" {
                    AcpChoice::Grok
                } else {
                    AcpChoice::Opencode
                };
            }
            if let Ok(custom) = serde_json::from_value::<CustomConfig>(value["custom"].clone()) {
                config.custom = custom;
            }
        }
    }
    config.custom.api_key = credential("CUSTOM_MODEL_API_KEY");
    if crate::selftest::requested() || std::env::var("EB_AGENT").ok().as_deref() == Some("demo") {
        config.default = DefaultRoute::Demo;
    } else if std::env::var("EB_AGENT").ok().as_deref() == Some("dsh") {
        config.default = DefaultRoute::Dsh;
    } else if let Ok(agent) = std::env::var("EVOBUDDY_ACP_AGENT") {
        config.default = DefaultRoute::Acp;
        config.acp = if agent == "grok" { AcpChoice::Grok } else { AcpChoice::Opencode };
    }
    config
}

fn credential(name: &str) -> String {
    if let Ok(text) = std::fs::read_to_string(credentials_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(key) = value[name].as_str() {
                return key.into();
            }
        }
    }
    std::env::var(name).unwrap_or_default()
}

fn save(config: &RouterConfig) -> Result<(), String> {
    let root = home().join(".evobuddy");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let value = serde_json::json!({
        "default": config.default.id(),
        "acp": config.acp.id(),
        "custom": {
            "name": config.custom.name,
            "base_url": config.custom.base_url,
            "model": config.custom.model,
        }
    });
    std::fs::write(
        config_path(),
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    )
    .map_err(|error| error.to_string())?;
    if !config.custom.api_key.trim().is_empty() {
        let mut credentials = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(credentials_path()).unwrap_or_else(|_| "{}".into()),
        )
        .unwrap_or_else(|_| serde_json::json!({}));
        credentials["CUSTOM_MODEL_API_KEY"] = serde_json::json!(config.custom.api_key.trim());
        std::fs::write(
            credentials_path(),
            serde_json::to_string_pretty(&credentials).unwrap_or_default(),
        )
        .map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(credentials_path(), std::fs::Permissions::from_mode(0o600));
        }
    }
    Ok(())
}

pub struct AgentRouter {
    config: RwLock<RouterConfig>,
    acp: Mutex<HashMap<AcpChoice, Arc<AcpAgent>>>,
    dsh: Arc<crate::dsh_agent::DshAgent>,
    demo: Arc<eb_plugin_demo_agent::DemoAgent>,
}

impl AgentRouter {
    pub fn new(config: RouterConfig) -> Self {
        Self {
            config: RwLock::new(config),
            acp: Mutex::new(HashMap::new()),
            dsh: Arc::new(crate::dsh_agent::DshAgent),
            demo: Arc::new(eb_plugin_demo_agent::DemoAgent::new()),
        }
    }

    pub fn config(&self) -> RouterConfig {
        self.config.read().unwrap().clone()
    }

    pub fn set_default(&self, route: DefaultRoute) -> Result<(), String> {
        let mut config = self.config.write().unwrap();
        config.default = route;
        save(&config)?;
        if route != DefaultRoute::Acp {
            // Idle ACP processes are dropped; a running turn keeps its own
            // clone until it finishes.
            self.acp.lock().unwrap().clear();
        }
        Ok(())
    }

    pub fn set_acp(&self, choice: AcpChoice) -> Result<(), String> {
        let mut config = self.config.write().unwrap();
        config.acp = choice;
        config.default = DefaultRoute::Acp;
        save(&config)?;
        self.acp.lock().unwrap().retain(|cached, _| *cached == choice);
        Ok(())
    }

    pub fn set_custom(&self, mut custom: CustomConfig) -> Result<(), String> {
        if custom.api_key.trim().is_empty() {
            custom.api_key = credential("CUSTOM_MODEL_API_KEY");
        }
        let mut config = self.config.write().unwrap();
        config.custom = custom;
        config.default = DefaultRoute::Custom;
        save(&config)?;
        self.acp.lock().unwrap().clear();
        Ok(())
    }

    fn acp_source(&self, choice: AcpChoice) -> Arc<AcpAgent> {
        let mut cache = self.acp.lock().unwrap();
        cache
            .entry(choice)
            .or_insert_with(|| {
                Arc::new(AcpAgent::new(AcpConfig {
                    program: choice.program().to_string_lossy().to_string(),
                    args: choice.args().iter().map(|arg| (*arg).to_string()).collect(),
                    cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                    auto_approve: std::env::var("EVOBUDDY_ACP_AUTO_APPROVE")
                        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
                        .unwrap_or(false),
                    name: choice.label().into(),
                    debug: std::env::var_os("EVOBUDDY_ACP_DEBUG").is_some(),
                    env: crate::agents::external_env(),
                    auth_method: std::env::var("EVOBUDDY_ACP_AUTH_METHOD").ok(),
                }))
            })
            .clone()
    }

    fn source(&self, config: &RouterConfig) -> Arc<dyn AnswerSource> {
        match config.default {
            DefaultRoute::Acp => self.acp_source(config.acp),
            DefaultRoute::Dsh => self.dsh.clone(),
            DefaultRoute::Custom => Arc::new(CustomModel::new(config.custom.clone())),
            DefaultRoute::Demo => self.demo.clone(),
        }
    }
}

impl AnswerSource for AgentRouter {
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        let config = self.config();
        let source = self.source(&config);
        source.start(prompt, sink)
    }

    fn start_in(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: Arc<dyn PermissionPrompter>,
        workspace: Option<PathBuf>,
    ) -> Box<dyn AnswerHandle> {
        let config = self.config();
        let source = self.source(&config);
        source.start_in(prompt, sink, permissions, workspace)
    }

    fn start_in_route(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: Arc<dyn PermissionPrompter>,
        workspace: Option<PathBuf>,
        route: Option<&str>,
    ) -> Box<dyn AnswerHandle> {
        let config = self.config();
        let source: Arc<dyn AnswerSource> = match route {
            Some("acp:opencode") => self.acp_source(AcpChoice::Opencode),
            Some("acp:grok") => self.acp_source(AcpChoice::Grok),
            Some("dsh") => self.dsh.clone(),
            Some("custom") => Arc::new(CustomModel::new(config.custom.clone())),
            _ => self.source(&config),
        };
        source.start_in(prompt, sink, permissions, workspace)
    }
}

static ROUTER: OnceLock<Arc<AgentRouter>> = OnceLock::new();

pub fn install(router: Arc<AgentRouter>) {
    let _ = ROUTER.set(router);
}

pub fn router() -> Option<Arc<AgentRouter>> {
    ROUTER.get().cloned()
}

pub fn set_default(route: DefaultRoute) -> Result<(), String> {
    match router() {
        Some(router) => router.set_default(route),
        None => Err("agent router is not installed yet".into()),
    }
}

pub fn set_acp(choice: AcpChoice) -> Result<(), String> {
    match router() {
        Some(router) => router.set_acp(choice),
        None => Err("agent router is not installed yet".into()),
    }
}

pub fn set_custom(config: CustomConfig) -> Result<(), String> {
    match router() {
        Some(router) => router.set_custom(config),
        None => Err("agent router is not installed yet".into()),
    }
}

pub fn current() -> RouterConfig {
    router().map(|router| router.config()).unwrap_or_else(load)
}

pub fn label() -> String {
    let config = current();
    match config.default {
        DefaultRoute::Acp => format!("ACP · {}", config.acp.label()),
        DefaultRoute::Dsh => format!("dsh · {}", crate::models::active_label()),
        DefaultRoute::Custom => format!(
            "Custom · {}",
            if config.custom.name.trim().is_empty() {
                config.custom.model
            } else {
                config.custom.name
            }
        ),
        DefaultRoute::Demo => "Demo".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_ids_round_trip() {
        for route in [DefaultRoute::Acp, DefaultRoute::Dsh, DefaultRoute::Custom, DefaultRoute::Demo] {
            assert_eq!(DefaultRoute::parse(route.id()), route);
        }
    }
}


/// Menu order shared by the composer selector and Settings labels.
pub fn menu_labels() -> Vec<&'static str> {
    vec![
        "Default",
        "ACP · opencode",
        "ACP · Grok Build",
        "dsh",
        "Custom Model",
    ]
}

pub fn route_for_menu(index: usize) -> Option<&'static str> {
    match index {
        1 => Some("acp:opencode"),
        2 => Some("acp:grok"),
        3 => Some("dsh"),
        4 => Some("custom"),
        _ => None,
    }
}

pub fn menu_index(route: Option<&str>) -> usize {
    match route {
        Some("acp:opencode") => 1,
        Some("acp:grok") => 2,
        Some("dsh") => 3,
        Some("custom") => 4,
        _ => 0,
    }
}

pub fn route_label(route: Option<&str>) -> String {
    match route {
        Some("acp:opencode") => "ACP · opencode".into(),
        Some("acp:grok") => "ACP · Grok Build".into(),
        Some("dsh") => "dsh".into(),
        Some("custom") => "Custom Model".into(),
        _ => format!("Default · {}", label()),
    }
}


/// Resolve a task override to a concrete route string.
pub fn effective_route(override_route: Option<&str>) -> String {
    if let Some(route) = override_route {
        return route.to_string();
    }
    let config = current();
    match config.default {
        DefaultRoute::Acp => format!("acp:{}", config.acp.id()),
        DefaultRoute::Dsh => "dsh".into(),
        DefaultRoute::Custom => "custom".into(),
        DefaultRoute::Demo => "demo".into(),
    }
}
