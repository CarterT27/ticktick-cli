use anyhow::{Context, Result};
use directories::ProjectDirs;
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Arc;

pub mod auth;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

impl Config {
    pub fn is_access_token_expired(&self, now: i64) -> bool {
        self.expires_at <= now
    }

    pub fn update_tokens(&mut self, access_token: String, refresh_token: String, expires_at: i64) {
        self.access_token = access_token;
        if !refresh_token.is_empty() {
            self.refresh_token = refresh_token;
        }
        self.expires_at = expires_at;
    }
}

#[derive(Clone)]
pub struct AppConfig {
    config_file: PathBuf,
    token_store: Arc<dyn TokenStore>,
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConfig")
            .field("config_file", &self.config_file)
            .finish_non_exhaustive()
    }
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

        Ok(Self::with_token_store(
            config_file,
            Arc::new(KeyringTokenStore::default()),
        ))
    }

    pub fn load(&self) -> Result<Option<Config>> {
        if !self.config_file.exists() {
            return Ok(None);
        }

        let contents =
            fs::read_to_string(&self.config_file).context("Failed to read config file")?;

        let stored: StoredConfig =
            toml::from_str(&contents).context("Failed to parse config file")?;

        if let Some(config) = stored.legacy_config() {
            if let Err(err) = self.token_store.save(&StoredTokens::from_config(&config)) {
                if secure_storage_unavailable(&err) {
                    return Ok(Some(config));
                }

                return Err(err).context("Failed to migrate credentials to secure storage");
            }
            self.write_metadata(ConfigMetadata::from_config(&config))
                .context("Failed to rewrite config file without credentials")?;
            return Ok(Some(config));
        }

        let tokens = self
            .token_store
            .load()
            .context("Failed to load credentials from secure storage")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Config metadata exists but credentials are missing from secure storage"
                )
            })?;

        Ok(Some(tokens.into_config(stored.metadata())))
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        let tokens = StoredTokens::from_config(config);
        self.token_store
            .save(&tokens)
            .context("Failed to save credentials to secure storage")?;

        if let Err(err) = self.write_metadata(ConfigMetadata::from_config(config)) {
            let _ = self.token_store.clear();
            return Err(err);
        }

        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let had_legacy_config = self.has_legacy_plaintext_config()?;

        if self.config_file.exists() {
            fs::remove_file(&self.config_file).context("Failed to remove config file")?;
        }

        if let Err(err) = self.token_store.clear() {
            if had_legacy_config && secure_storage_unavailable(&err) {
                return Ok(());
            }

            return Err(err).context("Failed to clear credentials from secure storage");
        }

        Ok(())
    }

    pub fn config_file_path(&self) -> &PathBuf {
        &self.config_file
    }

    fn with_token_store(config_file: PathBuf, token_store: Arc<dyn TokenStore>) -> Self {
        Self {
            config_file,
            token_store,
        }
    }

    fn write_metadata(&self, metadata: ConfigMetadata) -> Result<()> {
        let contents =
            toml::to_string_pretty(&metadata).context("Failed to serialize config metadata")?;

        fs::write(&self.config_file, contents).context("Failed to write config file")
    }

    fn has_legacy_plaintext_config(&self) -> Result<bool> {
        if !self.config_file.exists() {
            return Ok(false);
        }

        let contents =
            fs::read_to_string(&self.config_file).context("Failed to read config file")?;
        let stored: StoredConfig =
            toml::from_str(&contents).context("Failed to parse config file")?;
        Ok(stored.legacy_config().is_some())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigMetadata {
    expires_at: i64,
}

impl ConfigMetadata {
    fn from_config(config: &Config) -> Self {
        Self {
            expires_at: config.expires_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredConfig {
    expires_at: i64,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

impl StoredConfig {
    fn metadata(self) -> ConfigMetadata {
        ConfigMetadata {
            expires_at: self.expires_at,
        }
    }

    fn legacy_config(&self) -> Option<Config> {
        let access_token = self.access_token.clone()?;
        let refresh_token = self.refresh_token.clone()?;

        Some(Config {
            access_token,
            refresh_token,
            expires_at: self.expires_at,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredTokens {
    access_token: String,
    refresh_token: String,
}

impl StoredTokens {
    fn from_config(config: &Config) -> Self {
        Self {
            access_token: config.access_token.clone(),
            refresh_token: config.refresh_token.clone(),
        }
    }

    fn into_config(self, metadata: ConfigMetadata) -> Config {
        Config {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at: metadata.expires_at,
        }
    }
}

trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<StoredTokens>>;
    fn save(&self, tokens: &StoredTokens) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

#[derive(Debug, Default)]
struct KeyringTokenStore;

impl KeyringTokenStore {
    const SERVICE: &'static str = "ticktick-cli";
    const ACCOUNT: &'static str = "oauth";

    fn entry(&self) -> Result<Entry> {
        Entry::new(Self::SERVICE, Self::ACCOUNT).context("Failed to initialize keyring entry")
    }
}

impl TokenStore for KeyringTokenStore {
    fn load(&self) -> Result<Option<StoredTokens>> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(value) => serde_json::from_str(&value)
                .context("Failed to parse credentials from secure storage")
                .map(Some),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(err) => Err(err).context("Failed to read credentials from secure storage"),
        }
    }

    fn save(&self, tokens: &StoredTokens) -> Result<()> {
        let entry = self.entry()?;
        let serialized =
            serde_json::to_string(tokens).context("Failed to serialize secure credentials")?;
        entry
            .set_password(&serialized)
            .context("Failed to write credentials to secure storage")
    }

    fn clear(&self) -> Result<()> {
        let entry = self.entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(err) => Err(err).context("Failed to delete credentials from secure storage"),
        }
    }
}

fn secure_storage_unavailable(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<KeyringError>()
            .is_some_and(|keyring_err| {
                matches!(
                    keyring_err,
                    KeyringError::PlatformFailure(_) | KeyringError::NoStorageAccess(_)
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::io;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_config_path() -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "ticktick-cli-config-test-{}-{}",
            std::process::id(),
            TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[derive(Debug, Default)]
    struct MemoryTokenStore {
        tokens: Mutex<Option<StoredTokens>>,
    }

    impl TokenStore for MemoryTokenStore {
        fn load(&self) -> Result<Option<StoredTokens>> {
            Ok(self.tokens.lock().unwrap().clone())
        }

        fn save(&self, tokens: &StoredTokens) -> Result<()> {
            *self.tokens.lock().unwrap() = Some(tokens.clone());
            Ok(())
        }

        fn clear(&self) -> Result<()> {
            *self.tokens.lock().unwrap() = None;
            Ok(())
        }
    }

    fn test_app_config(path: PathBuf) -> AppConfig {
        AppConfig::with_token_store(path, Arc::new(MemoryTokenStore::default()))
    }

    #[derive(Debug)]
    struct ErrorTokenStore {
        save_error: Option<String>,
        clear_error: Option<String>,
    }

    impl ErrorTokenStore {
        fn secure_storage_error(message: &str) -> anyhow::Error {
            anyhow::Error::new(KeyringError::NoStorageAccess(Box::new(io::Error::new(
                io::ErrorKind::Other,
                message.to_string(),
            ))))
        }
    }

    impl TokenStore for ErrorTokenStore {
        fn load(&self) -> Result<Option<StoredTokens>> {
            Ok(None)
        }

        fn save(&self, _tokens: &StoredTokens) -> Result<()> {
            match &self.save_error {
                Some(message) => Err(Self::secure_storage_error(message)),
                None => Ok(()),
            }
        }

        fn clear(&self) -> Result<()> {
            match &self.clear_error {
                Some(message) => Err(Self::secure_storage_error(message)),
                None => Ok(()),
            }
        }
    }

    #[test]
    fn load_returns_none_when_config_file_is_missing() {
        let path = temp_config_path();
        let app_config = test_app_config(path.clone());

        assert_eq!(app_config.config_file_path(), &path);
        assert!(app_config.load().unwrap().is_none());
    }

    #[test]
    fn save_load_and_clear_round_trip_config_without_plaintext_tokens() {
        let path = temp_config_path();
        let app_config = test_app_config(path.clone());
        let expected = Config {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_at: 123456789,
        };

        app_config.save(&expected).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("expires_at = 123456789"));
        assert!(!contents.contains("access-token"));
        assert!(!contents.contains("refresh-token"));

        let loaded = app_config.load().unwrap().unwrap();
        assert_eq!(loaded.access_token, expected.access_token);
        assert_eq!(loaded.refresh_token, expected.refresh_token);
        assert_eq!(loaded.expires_at, expected.expires_at);

        app_config.clear().unwrap();
        assert!(!path.exists());
        assert!(app_config.load().unwrap().is_none());
    }

    #[test]
    fn load_migrates_legacy_plaintext_credentials_into_secure_storage() {
        let path = temp_config_path();
        let app_config = test_app_config(path.clone());

        fs::write(
            &path,
            r#"
access_token = "legacy-access"
refresh_token = "legacy-refresh"
expires_at = 987654321
"#,
        )
        .unwrap();

        let loaded = app_config.load().unwrap().unwrap();
        assert_eq!(loaded.access_token, "legacy-access");
        assert_eq!(loaded.refresh_token, "legacy-refresh");
        assert_eq!(loaded.expires_at, 987654321);

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("expires_at = 987654321"));
        assert!(!contents.contains("legacy-access"));
        assert!(!contents.contains("legacy-refresh"));
    }

    #[test]
    fn load_keeps_legacy_plaintext_credentials_when_secure_storage_is_unavailable() {
        let path = temp_config_path();
        let app_config = AppConfig::with_token_store(
            path.clone(),
            Arc::new(ErrorTokenStore {
                save_error: Some("secure storage unavailable".to_string()),
                clear_error: None,
            }),
        );

        fs::write(
            &path,
            r#"
access_token = "legacy-access"
refresh_token = "legacy-refresh"
expires_at = 987654321
"#,
        )
        .unwrap();

        let loaded = app_config.load().unwrap().unwrap();
        assert_eq!(loaded.access_token, "legacy-access");
        assert_eq!(loaded.refresh_token, "legacy-refresh");
        assert_eq!(loaded.expires_at, 987654321);

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("legacy-access"));
        assert!(contents.contains("legacy-refresh"));
    }

    #[test]
    fn clear_succeeds_for_legacy_plaintext_config_when_secure_storage_is_unavailable() {
        let path = temp_config_path();
        let app_config = AppConfig::with_token_store(
            path.clone(),
            Arc::new(ErrorTokenStore {
                save_error: None,
                clear_error: Some("secure storage unavailable".to_string()),
            }),
        );

        fs::write(
            &path,
            r#"
access_token = "legacy-access"
refresh_token = "legacy-refresh"
expires_at = 987654321
"#,
        )
        .unwrap();

        app_config.clear().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn access_token_expiration_check_uses_expires_at() {
        let config = Config {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_at: 100,
        };

        assert!(config.is_access_token_expired(100));
        assert!(config.is_access_token_expired(101));
        assert!(!config.is_access_token_expired(99));
    }

    #[test]
    fn update_tokens_preserves_refresh_token_when_refresh_response_omits_it() {
        let mut config = Config {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_at: 100,
        };

        config.update_tokens("new-access-token".to_string(), String::new(), 200);

        assert_eq!(config.access_token, "new-access-token");
        assert_eq!(config.refresh_token, "refresh-token");
        assert_eq!(config.expires_at, 200);
    }
}
