use crate::api::TickTickClient;
use crate::config::{AppConfig, Config};
use anyhow::{anyhow, Result};

const NOT_AUTHENTICATED_MESSAGE: &str = "Not authenticated. Run 'tt auth login' first.";

pub fn app_config() -> Result<AppConfig> {
    AppConfig::new()
}

pub fn load_config() -> Result<Option<Config>> {
    app_config()?.load()
}

pub fn require_config() -> Result<Config> {
    load_config()?.ok_or_else(|| anyhow!(NOT_AUTHENTICATED_MESSAGE))
}

pub fn authenticated_client() -> Result<TickTickClient> {
    TickTickClient::new(require_config()?)
}
