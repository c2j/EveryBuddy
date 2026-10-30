//! The live answer-source router.
//!
//! Host::mount installs this router once. Settings update its route in place;
//! each new turn resolves the current default without rebuilding GTK or the
//! rutis host. Existing turns keep the source they started on.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use eb_api::{AgentSink, AnswerHandle, AnswerSource, PermissionPrompter};
use eb_plugin_acp_agent::{AcpAgent, AcpConfig, TailPolicy};

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
        let acp = env_agent
            .as_deref()
            .and_then(AcpChoice::parse)
            .unwrap_or(AcpChoice::Opencode);
        let default = if std::env::var("EB_AGENT").ok().as_deref() == Some("demo")
            || crate::selftest::requested()
        {
            DefaultRoute::Demo
        } else if std::env::var("EB_AGENT").ok().as_deref() == Some("dsh") {
            DefaultRoute::Dsh
        } else {
            DefaultRoute::Acp
        };
        Self {
            default,
            acp,
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
    if std::env::var_os("EB_DEBUG").is_some() {
        eprintln!(
            "router: loading {}",
            config_path().display()
        );
    }
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(default) = value["default"].as_str() {
                config.default = DefaultRoute::parse(default);
            }
            if let Some(acp) = value["acp"].as_str().and_then(AcpChoice::parse) {
                config.acp = acp;
            }
            set_permission_mode(value["permission_mode"].as_u64().unwrap_or(0) as usize);
            match serde_json::from_value::<CustomConfig>(value["custom"].clone()) {
                Ok(custom) => config.custom = custom,
                // A failed parse must not masquerade as "no custom model":
                // router.json deliberately omits api_key (it lives in
                // credentials.json), which used to fail the whole struct and
                // silently wipe the saved model on every restart.
                Err(error) if value.get("custom").is_some() => {
                    if std::env::var_os("EB_DEBUG").is_some() {
                        eprintln!("router: custom config failed to parse: {error}");
                    }
                }
                _ => {}
            }
        }
    }
    config.custom.api_key = credential("CUSTOM_MODEL_API_KEY");
    // An explicit agent choice outranks the harness default: EB_SHOT and
    // friends force Demo so screenshots are deterministic, but the ACP
    // verification recipe sets EVOBUDDY_ACP_AGENT precisely to *not* get Demo.
    if let Ok(agent) = std::env::var("EVOBUDDY_ACP_AGENT") {
        config.default = DefaultRoute::Acp;
        if let Some(choice) = AcpChoice::parse(&agent) {
            config.acp = choice;
        }
    } else if crate::selftest::requested()
        || std::env::var("EB_AGENT").ok().as_deref() == Some("demo")
    {
        config.default = DefaultRoute::Demo;
    } else if std::env::var("EB_AGENT").ok().as_deref() == Some("dsh") {
        config.default = DefaultRoute::Dsh;
    }
    if std::env::var_os("EB_DEBUG").is_some() {
        eprintln!(
            "router: default={} custom={{name: {:?}, base_url: {:?}, model: {:?}, api_key: {}}}",
            config.default.id(),
            config.custom.name,
            config.custom.base_url,
            config.custom.model,
            if config.custom.api_key.is_empty() { "missing" } else { "present" }
        );
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
        "permission_mode": PERMISSION_MODE.load(Ordering::SeqCst),
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

/// The spawn configuration for one ACP slot. Shared by the router's session
/// cache and the `EB_ACP_PROBE` harness, so a probe tests exactly what a real
/// turn would run.
pub fn acp_config(choice: AcpChoice) -> AcpConfig {
    let (program, args) = choice.launch();
    AcpConfig {
        program: program.to_string_lossy().to_string(),
        args,
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        auto_approve: std::env::var("EVOBUDDY_ACP_AUTO_APPROVE")
            .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false),
        name: choice.label().into(),
        debug: std::env::var_os("EVOBUDDY_ACP_DEBUG").is_some(),
        env: crate::agents::external_env(),
        auth_method: std::env::var("EVOBUDDY_ACP_AUTH_METHOD").ok(),
        tail: TailPolicy::from_env(),
        log: AcpConfig::log_from_env(),
    }
}

fn acp_source(choice: AcpChoice) -> Arc<AcpAgent> {
    Arc::new(AcpAgent::new(acp_config(choice)))
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
        cache.entry(choice).or_insert_with(|| acp_source(choice)).clone()
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
            Some(route) => match route.strip_prefix("acp:").and_then(AcpChoice::parse) {
                Some(choice) => self.acp_source(choice),
                None => self.fallback(route, &config),
            },
            None => self.source(&config),
        };
        source.start_in(prompt, sink, permissions, workspace)
    }
}

impl AgentRouter {
    /// The non-ACP route ids; anything unknown keeps the configured default.
    fn fallback(&self, route: &str, config: &RouterConfig) -> Arc<dyn AnswerSource> {
        match route {
            "dsh" => self.dsh.clone(),
            "custom" => Arc::new(CustomModel::new(config.custom.clone())),
            _ => self.source(config),
        }
    }
}

static ROUTER: OnceLock<Arc<AgentRouter>> = OnceLock::new();

/// The composer's ◇ permission picker (0 Default Permissions, 1 Auto-approve,
/// 2 Read-only). Process-global because tool workers reach it from their own
/// threads at ask time; persisted in router.json.
static PERMISSION_MODE: AtomicUsize = AtomicUsize::new(0);

pub fn permission_mode() -> usize {
    PERMISSION_MODE.load(Ordering::SeqCst)
}

pub fn set_permission_mode(mode: usize) {
    PERMISSION_MODE.store(mode.min(2), Ordering::SeqCst);
}

/// Remember the composer ◇ choice across restarts. `load` only updates the
/// in-memory mode; the picker is what writes `router.json`.
pub fn persist_permission_mode(mode: usize) {
    set_permission_mode(mode);
    if let Some(router) = router() {
        let config = router.config();
        let _ = save(&config);
    }
}

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

    /// The exact restart scenario from the field: router.json is saved
    /// without api_key (it lives in credentials.json), and load() must
    /// restore the custom model and re-attach the key.
    #[test]
    fn custom_config_survives_a_restart() {
        let temp = std::env::temp_dir().join(format!("eb-router-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(temp.join(".evobuddy")).unwrap();
        std::fs::write(
            temp.join(".evobuddy/router.json"),
            r#"{"acp":"opencode","custom":{"base_url":"https://open.bigmodel.cn/api/coding/paas/v4","model":"glm-5.3-flash","name":"GLM2"},"default":"custom"}"#,
        )
        .unwrap();
        std::fs::write(
            temp.join(".evobuddy/credentials.json"),
            r#"{"CUSTOM_MODEL_API_KEY":"k-123"}"#,
        )
        .unwrap();

        let previous_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp);
        let config = load();
        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }

        assert_eq!(config.default, DefaultRoute::Custom);
        assert_eq!(config.custom.name, "GLM2");
        assert_eq!(config.custom.model, "glm-5.3-flash");
        assert_eq!(config.custom.base_url, "https://open.bigmodel.cn/api/coding/paas/v4");
        assert_eq!(config.custom.api_key, "k-123");
        std::fs::remove_dir_all(&temp).unwrap();
    }

    #[test]
    fn route_ids_round_trip() {
        for route in [DefaultRoute::Acp, DefaultRoute::Dsh, DefaultRoute::Custom, DefaultRoute::Demo] {
            assert_eq!(DefaultRoute::parse(route.id()), route);
        }
    }

    /// The route menu is built from a table, so a new ACP slot has to show up in
    /// it — and the labels must be the ones `AcpAgent` reports.
    #[test]
    fn the_route_menu_lists_every_acp_slot() {
        for (position, agent) in agents::all().iter().enumerate() {
            let (route, label) = ACP_MENU[position];
            assert_eq!(route, format!("acp:{}", agent.id()));
            assert_eq!(label, format!("ACP · {}", agent.label()));
        }
        assert_eq!(menu_labels().len(), ACP_MENU.len() + 3);
        for index in 0..menu_labels().len() {
            let route = route_for_menu(index).map(str::to_string);
            assert_eq!(menu_index(route.as_deref()), index, "menu index {index}");
        }
        assert_eq!(route_label(Some("acp:jcode")), "ACP · jcode");
        assert_eq!(route_label(Some("acp:pi")), "ACP · pi");
        assert_eq!(menu_index(Some("acp:gone")), 0);
    }
}


/// The ACP rows of the route menu, in menu order: route id plus label. Keeping
/// them in one table is what stops the composer's index-based lookup and
/// `route_label` from drifting apart when a slot is added.
const ACP_MENU: [(&str, &str); 4] = [
    ("acp:opencode", "ACP · opencode"),
    ("acp:grok", "ACP · Grok Build"),
    ("acp:jcode", "ACP · jcode"),
    ("acp:pi", "ACP · pi"),
];
const DSH_MENU_INDEX: usize = ACP_MENU.len() + 1;
const CUSTOM_MENU_INDEX: usize = DSH_MENU_INDEX + 1;

/// Menu order shared by the composer selector and Settings labels.
pub fn menu_labels() -> Vec<&'static str> {
    let mut labels = vec!["Default"];
    labels.extend(ACP_MENU.iter().map(|(_, label)| *label));
    labels.extend(["dsh", "Custom Model"]);
    labels
}

pub fn route_for_menu(index: usize) -> Option<&'static str> {
    match index {
        0 => None,
        i if i <= ACP_MENU.len() => Some(ACP_MENU[i - 1].0),
        i if i == DSH_MENU_INDEX => Some("dsh"),
        i if i == CUSTOM_MENU_INDEX => Some("custom"),
        _ => None,
    }
}

pub fn menu_index(route: Option<&str>) -> usize {
    let route = match route {
        Some(route) => route,
        None => return 0,
    };
    ACP_MENU
        .iter()
        .position(|(id, _)| *id == route)
        .map_or_else(
            || match route {
                "dsh" => DSH_MENU_INDEX,
                "custom" => CUSTOM_MENU_INDEX,
                _ => 0,
            },
            |position| position + 1,
        )
}

pub fn route_label(route: Option<&str>) -> String {
    match route {
        Some("dsh") => "dsh".into(),
        Some("custom") => {
            let custom = current().custom;
            if custom.name.trim().is_empty() {
                "Custom Model".into()
            } else {
                custom.name
            }
        }
        Some(route) => ACP_MENU
            .iter()
            .find(|(id, _)| *id == route)
            .map(|(_, label)| (*label).to_string())
            .unwrap_or_else(|| format!("Default · {}", label())),
        None => format!("Default · {}", label()),
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
