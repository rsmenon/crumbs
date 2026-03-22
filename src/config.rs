use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_theme")]
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            theme: default_theme(),
        }
    }
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn default_data_dir() -> PathBuf {
    home_dir().join(".crumb")
}

fn default_theme() -> String {
    "gruvbox_dark".to_string()
}

fn config_path() -> PathBuf {
    home_dir().join(".crumb").join("settings.yaml")
}

pub fn load() -> Result<Config> {
    let path = config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        let cfg: Config = serde_yaml::from_str(&content)?;
        Ok(cfg)
    } else {
        let cfg = Config::default();
        // Create ~/.crumb/ and write default settings
        std::fs::create_dir_all(&cfg.data_dir)?;
        let yaml = serde_yaml::to_string(&cfg)?;
        std::fs::write(&path, yaml)?;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_gruvbox_theme() {
        let cfg = Config::default();
        assert_eq!(cfg.theme, "gruvbox_dark");
    }

    #[test]
    fn default_data_dir_is_dot_crumb() {
        let cfg = Config::default();
        assert!(cfg.data_dir.ends_with(".crumb"));
    }

    #[test]
    fn config_path_inside_data_dir() {
        let path = config_path();
        assert!(path.ends_with(".crumb/settings.yaml"));
    }

    #[test]
    fn roundtrip_yaml() {
        let cfg = Config::default();
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let cfg2: Config = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(cfg.theme, cfg2.theme);
        assert_eq!(cfg.data_dir, cfg2.data_dir);
    }
}
