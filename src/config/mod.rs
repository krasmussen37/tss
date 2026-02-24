use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Per-source configuration block from config.toml.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct SourceConfig {
    pub api_key: Option<String>,
    pub api_key_command: Option<String>,
    pub default_tag: Option<String>,
    pub base_url: Option<String>,
}

/// Top-level tss config file structure.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct TssConfig {
    pub fireflies: Option<SourceConfig>,
    pub pocket: Option<SourceConfig>,
}

impl TssConfig {
    /// Load config from ~/.tss/config.toml. Returns default if file doesn't exist.
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(TssConfig::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;
        let config: TssConfig =
            toml::from_str(&content).with_context(|| "Failed to parse config.toml")?;
        Ok(config)
    }

    /// Get source config by name.
    pub fn source_config(&self, source: &str) -> Option<&SourceConfig> {
        match source {
            "fireflies" => self.fireflies.as_ref(),
            "pocket" => self.pocket.as_ref(),
            _ => None,
        }
    }

    /// Display config with secrets redacted.
    pub fn display_redacted(&self) -> String {
        let mut lines = Vec::new();
        if let Some(ref ff) = self.fireflies {
            lines.push("[fireflies]".to_string());
            display_source_config(&mut lines, ff);
        }
        if let Some(ref pk) = self.pocket {
            lines.push("[pocket]".to_string());
            display_source_config(&mut lines, pk);
        }
        if lines.is_empty() {
            lines.push("(no sources configured)".to_string());
        }
        lines.join("\n")
    }
}

fn display_source_config(lines: &mut Vec<String>, sc: &SourceConfig) {
    if let Some(ref key) = sc.api_key {
        let redacted = if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len() - 4..])
        } else {
            "****".to_string()
        };
        lines.push(format!("  api_key = \"{}\"", redacted));
    }
    if let Some(ref cmd) = sc.api_key_command {
        lines.push(format!("  api_key_command = \"{}\"", cmd));
    }
    if let Some(ref tag) = sc.default_tag {
        lines.push(format!("  default_tag = \"{}\"", tag));
    }
    if let Some(ref url) = sc.base_url {
        lines.push(format!("  base_url = \"{}\"", url));
    }
}

/// Resolve a credential through the chain: CLI flag > env var > config key > config command.
pub fn resolve_credential(
    cli_flag: Option<&str>,
    env_var_name: &str,
    config: Option<&SourceConfig>,
) -> Result<String> {
    // 1. CLI flag
    if let Some(key) = cli_flag {
        if !key.is_empty() {
            return Ok(key.to_string());
        }
    }

    // 2. Environment variable
    if let Ok(val) = std::env::var(env_var_name) {
        if !val.is_empty() {
            return Ok(val);
        }
    }

    if let Some(sc) = config {
        // 3. Config file api_key
        if let Some(ref key) = sc.api_key {
            if !key.is_empty() {
                return Ok(key.clone());
            }
        }

        // 4. External command
        if let Some(ref cmd) = sc.api_key_command {
            if !cmd.is_empty() {
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
                    .with_context(|| format!("Failed to run api_key_command: {cmd}"))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!(
                        "api_key_command failed (exit {}): {}",
                        output.status.code().unwrap_or(-1),
                        stderr.trim()
                    );
                }

                let secret = String::from_utf8(output.stdout)
                    .context("api_key_command output is not valid UTF-8")?
                    .trim()
                    .to_string();

                if !secret.is_empty() {
                    return Ok(secret);
                }
            }
        }
    }

    bail!(
        "No API key found. Provide via --api-key, {} env var, or ~/.tss/config.toml",
        env_var_name
    );
}

/// Path to the config file: ~/.tss/config.toml
pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".tss").join("config.toml"))
}

/// Default config template content.
pub fn default_config_template() -> &'static str {
    r#"# ~/.tss/config.toml
# Credential resolution order: CLI flag > env var > api_key > api_key_command

[fireflies]
# api_key = "your-fireflies-api-key"
# api_key_command = "your-secrets-manager-command-here"

[pocket]
# api_key = "your-pocket-api-key"
# api_key_command = "your-secrets-manager-command-here"
# default_tag = "your-tag-name"
"#
}

/// Create the default config file if it doesn't already exist.
pub fn init_config() -> Result<bool> {
    let path = config_path()?;
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, default_config_template())?;
    Ok(true)
}
