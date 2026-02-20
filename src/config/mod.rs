use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;

pub mod auth;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    config_file: PathBuf,
}

impl AppConfig {
    pub fn new() -> Result<Self> {
        let proj_dirs = ProjectDirs::from("", "", "ticktick-cli")
            .context("Failed to get project directories")?;

        let config_dir = proj_dirs.config_dir().to_path_buf();

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir).context("Failed to create config directory")?;
        }

        let config_file = config_dir.join("config.toml");

        Ok(AppConfig { config_file })
    }

    pub fn load(&self) -> Result<Option<Config>> {
        if !self.config_file.exists() {
            return Ok(None);
        }

        let contents =
            fs::read_to_string(&self.config_file).context("Failed to read config file")?;

        let config: Config = toml::from_str(&contents).context("Failed to parse config file")?;

        Ok(Some(config))
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        let contents = toml::to_string_pretty(config).context("Failed to serialize config")?;

        fs::write(&self.config_file, contents).context("Failed to write config file")?;

        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        if self.config_file.exists() {
            fs::remove_file(&self.config_file).context("Failed to remove config file")?;
        }
        Ok(())
    }

    pub fn config_file_path(&self) -> &PathBuf {
        &self.config_file
    }
}
