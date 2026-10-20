//! Native model settings for the dsh path.
//!
//! The dsh Web client ships `@deepseek-ai/dsh-client-ui-settings-models`, a
//! React client plugin that talks to dsh's settings/credentials remotes. EvoBuddy
//! is a GTK host, so it cannot mount that plugin directly. This module keeps the
//! same concept native: a provider/model pair, endpoint, and write-only API key,
//! persisted under `~/.evobuddy` and applied to the dsh bridge's AIMUX_* env.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn config_path() -> PathBuf {
    home().join(".evobuddy/models.json")
}

fn credentials_path() -> PathBuf {
    home().join(".evobuddy/credentials.json")
}

fn env_name(provider: &str) -> String {
    let mut name: String = provider
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' })
        .collect();
    if name.is_empty() {
        name = "CUSTOM".into();
    }
    format!("{name}_API_KEY")
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: std::env::var("AIMUX_PROVIDER").unwrap_or_else(|_| "aihubmix".into()),
            model: std::env::var("AIMUX_MODEL").unwrap_or_else(|_| "deepseek-v4.1-flash".into()),
            base_url: "https://aihubmix.com/v1".into(),
            api_key: String::new(),
        }
    }
}

pub fn load() -> ModelConfig {
    let mut config = ModelConfig::default();
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(provider) = value["provider"].as_str() {
                config.provider = provider.into();
            }
            if let Some(model) = value["model"].as_str() {
                config.model = model.into();
            }
            if let Some(base_url) = value["base_url"].as_str() {
                config.base_url = base_url.into();
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(credentials_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(key) = value[env_name(&config.provider)].as_str() {
                config.api_key = key.into();
            }
        }
    }
    config
}

pub fn save(config: &ModelConfig) -> Result<(), String> {
    if config.provider.trim().is_empty() || config.model.trim().is_empty() {
        return Err("provider and model are required".into());
    }
    let root = home().join(".evobuddy");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let value = serde_json::json!({
        "provider": config.provider.trim(),
        "model": config.model.trim(),
        "base_url": config.base_url.trim(),
    });
    std::fs::write(
        config_path(),
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    )
    .map_err(|error| error.to_string())?;
    if !config.api_key.trim().is_empty() {
        let mut credentials = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(credentials_path()).unwrap_or_else(|_| "{}".into()),
        )
        .unwrap_or_else(|_| serde_json::json!({}));
        credentials[env_name(&config.provider)] = serde_json::json!(config.api_key.trim());
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
    apply_env(&load());
    Ok(())
}

/// Mirror the saved config into the environment read by the live dsh bridge.
/// A running bridge keeps the env it was spawned with, so callers stop it when
/// they need the new model to take effect immediately.
pub fn apply_env(config: &ModelConfig) {
    std::env::set_var("AIMUX_PROVIDER", config.provider.trim());
    std::env::set_var("AIMUX_MODEL", config.model.trim());
    if !config.base_url.trim().is_empty() {
        std::env::set_var("AIMUX_BASE_URL", config.base_url.trim());
    }
    if !config.api_key.trim().is_empty() {
        std::env::set_var(env_name(&config.provider), config.api_key.trim());
    }
}

/// Model labels for the composer menu: Auto plus the configured model.
pub fn menu_labels() -> Vec<&'static str> {
    let config = load();
    vec!["Auto", Box::leak(config.model.into_boxed_str())]
}

/// Selecting a composer label does not change Auto; selecting the configured
/// model explicitly is the same model Auto resolves to today.
pub fn select(_index: usize) {}

pub fn active_label() -> String {
    let config = load();
    format!("{} · {}", config.provider, config.model)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_provider_or_model_is_rejected() {
        let mut config = ModelConfig::default();
        config.provider.clear();
        assert!(save(&config).is_err());
        let config = ModelConfig {
            provider: "aihubmix".into(),
            model: String::new(),
            ..ModelConfig::default()
        };
        assert!(save(&config).is_err());
    }

    #[test]
    fn provider_specific_key_names_are_stable() {
        assert_eq!(env_name("aihubmix"), "AIHUBMIX_API_KEY");
        assert_eq!(env_name("my-relay"), "MY_RELAY_API_KEY");
    }
}
