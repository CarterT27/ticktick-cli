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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path() -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "ticktick-cli-config-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn load_returns_none_when_config_file_is_missing() {
        let path = temp_config_path();
        let app_config = AppConfig {
            config_file: path.clone(),
        };

        assert_eq!(app_config.config_file_path(), &path);
        assert!(app_config.load().unwrap().is_none());
    }

    #[test]
    fn save_load_and_clear_round_trip_config() {
        let path = temp_config_path();
        let app_config = AppConfig {
            config_file: path.clone(),
        };
        let expected = Config {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_at: 123456789,
        };

        app_config.save(&expected).unwrap();
        let loaded = app_config.load().unwrap().unwrap();
        assert_eq!(loaded.access_token, expected.access_token);
        assert_eq!(loaded.refresh_token, expected.refresh_token);
        assert_eq!(loaded.expires_at, expected.expires_at);

        app_config.clear().unwrap();
        assert!(!path.exists());
        assert!(app_config.load().unwrap().is_none());
    }
}
