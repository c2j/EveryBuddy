//! Native model settings for the dsh path.
//!
//! The dsh Web client ships `@deepseek-ai/dsh-client-ui-settings-models`, a
//! React client plugin that talks to dsh's settings/credentials remotes. EvoBuddy
//! is a GTK host, so it cannot mount that plugin directly. This module keeps the
//! same concept native: a provider list with model ids and write-only API keys,
//! persisted under `~/.evobuddy` and applied to the dsh bridge's AIMUX_* env.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub models: Vec<String>,
    pub custom: bool,
}

#[derive(Clone, Debug)]
pub struct ModelConfig {
    pub provider: String,
    pub model: String,
    pub providers: Vec<Provider>,
    /// Transient value entered in the editor. Never written into models.json.
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

fn dsh_credentials_path() -> PathBuf {
    home().join(".dsh/.credentials.yaml")
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

fn env_aliases(provider: &str) -> Vec<String> {
    let primary = env_name(provider);
    let mut names = vec![primary.clone()];
    if provider == "zai-coding-cn" {
        names.push("ZAI_API_KEY".into());
    }
    names
}

fn default_providers() -> Vec<Provider> {
    vec![
        Provider {
            id: "deepseek".into(),
            models: vec!["deepseek-chat".into(), "deepseek-reasoner".into()],
            custom: false,
        },
        Provider {
            id: "zai-coding-cn".into(),
            models: vec!["glm-4.6".into()],
            custom: false,
        },
        Provider {
            id: "aihubmix".into(),
            models: vec!["deepseek-v4.1-flash".into()],
            custom: true,
        },
    ]
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: std::env::var("AIMUX_PROVIDER").unwrap_or_else(|_| "aihubmix".into()),
            model: std::env::var("AIMUX_MODEL").unwrap_or_else(|_| "deepseek-v4.1-flash".into()),
            providers: default_providers(),
            api_key: String::new(),
        }
    }
}

pub fn load() -> ModelConfig {
    let mut config = ModelConfig::default();
    if let Ok(text) = std::fs::read_to_string(config_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(provider) = value["active_provider"].as_str() {
                config.provider = provider.into();
            } else if let Some(provider) = value["provider"].as_str() {
                config.provider = provider.into();
            }
            if let Some(model) = value["active_model"].as_str() {
                config.model = model.into();
            } else if let Some(model) = value["model"].as_str() {
                config.model = model.into();
            }
            if let Some(providers) = value["providers"].as_array() {
                let loaded: Vec<Provider> = providers
                    .iter()
                    .filter_map(|row| {
                        let id = row["id"].as_str()?.to_string();
                        let mut models: Vec<String> = row["models"]
                            .as_array()
                            .map(|models| {
                                models
                                    .iter()
                                    .filter_map(|m| m.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        if models.is_empty() {
                            models.push(config.model.clone());
                        }
                        Some(Provider {
                            id,
                            models,
                            custom: row["custom"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect();
                if !loaded.is_empty() {
                    config.providers = loaded;
                }
            }
        }
    }
    config.api_key = api_key(&config.provider);
    config
}

pub fn api_key(provider: &str) -> String {
    if let Ok(text) = std::fs::read_to_string(credentials_path()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            for name in env_aliases(provider) {
                if let Some(key) = value[&name].as_str() {
                    if !key.trim().is_empty() {
                        return key.into();
                    }
                }
            }
        }
    }
    for name in env_aliases(provider) {
        if let Ok(key) = std::env::var(&name) {
            if !key.trim().is_empty() {
                return key;
            }
        }
    }
    // Reuse the dsh credential file when EvoBuddy has none of its own.
    if let Ok(text) = std::fs::read_to_string(dsh_credentials_path()) {
        let name = env_name(provider);
        let mapped = if provider == "zai-coding-cn" { "ZAI_CODING_CN_API_KEY:" } else { "" };
        for line in text.lines() {
            let line = line.trim();
            if (line.starts_with(&name) || (!mapped.is_empty() && line.starts_with(mapped)))
                && line.contains(':')
            {
                let value = line.split_once(':').map(|(_, v)| v).unwrap_or("").trim();
                let value = value.trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    return value.into();
                }
            }
        }
    }
    String::new()
}

pub fn has_key(provider: &str) -> bool {
    !api_key(provider).is_empty()
}

pub fn save(config: &ModelConfig) -> Result<(), String> {
    if config.provider.trim().is_empty() || config.model.trim().is_empty() {
        return Err("provider and model are required".into());
    }
    let root = home().join(".evobuddy");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let providers: Vec<serde_json::Value> = config
        .providers
        .iter()
        .map(|provider| {
            serde_json::json!({
                "id": provider.id,
                "models": provider.models,
                "custom": provider.custom,
            })
        })
        .collect();
    let value = serde_json::json!({
        "active_provider": config.provider.trim(),
        "active_model": config.model.trim(),
        "providers": providers,
    });
    std::fs::write(
        config_path(),
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    )
    .map_err(|error| error.to_string())?;
    if !config.api_key.trim().is_empty() {
        set_api_key(&config.provider, &config.api_key)?;
    }
    apply_env(&load());
    Ok(())
}

pub fn set_api_key(provider: &str, key: &str) -> Result<(), String> {
    let mut credentials = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(credentials_path()).unwrap_or_else(|_| "{}".into()),
    )
    .unwrap_or_else(|_| serde_json::json!({}));
    credentials[env_name(provider)] = serde_json::json!(key.trim());
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
    Ok(())
}

pub fn delete_provider(config: &mut ModelConfig, provider: &str) -> Result<(), String> {
    config.providers.retain(|row| row.id != provider);
    if config.providers.is_empty() {
        return Err("at least one provider is required".into());
    }
    if config.provider == provider {
        config.provider = config.providers[0].id.clone();
        config.model = config.providers[0]
            .models
            .first()
            .cloned()
            .unwrap_or_default();
    }
    let mut credentials = serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(credentials_path()).unwrap_or_else(|_| "{}".into()),
    )
    .unwrap_or_else(|_| serde_json::json!({}));
    credentials[env_name(provider)] = serde_json::Value::Null;
    let _ = std::fs::write(
        credentials_path(),
        serde_json::to_string_pretty(&credentials).unwrap_or_default(),
    );
    save(config)
}

/// Mirror the saved config into the environment read by the live dsh bridge.
/// A running bridge keeps the env it was spawned with, so callers stop it when
/// they need the new model to take effect immediately.
pub fn apply_env(config: &ModelConfig) {
    std::env::set_var("AIMUX_PROVIDER", config.provider.trim());
    std::env::set_var("AIMUX_MODEL", config.model.trim());
    for name in env_aliases(&config.provider) {
        let key = api_key(&config.provider);
        if !key.is_empty() {
            std::env::set_var(name, key);
        }
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

pub fn select(_index: usize) {}

pub fn active_label() -> String {
    let config = load();
    format!("{} · {}", config.provider, config.model)
}

pub fn config_path_display() -> String {
    config_path().display().to_string()
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
        assert!(env_aliases("zai-coding-cn").contains(&"ZAI_API_KEY".into()));
    }
}
