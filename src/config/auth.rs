use super::{AppConfig, Config};
use anyhow::{anyhow, Context, Result};
use oauth2::{
    basic::BasicClient, reqwest::async_http_client, AuthUrl, AuthorizationCode, ClientId,
    ClientSecret, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_AUTH_URL: &str = "https://ticktick.com/oauth/authorize";
const DEFAULT_TOKEN_URL: &str = "https://ticktick.com/oauth/token";
pub const DEFAULT_REDIRECT_URI: &str = "http://localhost:8080/callback";
const DEFAULT_EXPIRES_IN_SECS: i64 = 3600;
const REFRESH_MARGIN_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSettings {
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    broker_url: Option<String>,
    broker_api_key: Option<String>,
}

impl AuthSettings {
    pub fn from_env() -> Result<Self> {
        Self::from_env_with(|key| std::env::var(key))
    }

    fn from_env_with<F>(get_var: F) -> Result<Self>
    where
        F: Fn(&str) -> std::result::Result<String, std::env::VarError>,
    {
        let client_id = required_env(&get_var, "TICKTICK_CLIENT_ID")?;
        let redirect_uri = optional_env(&get_var, "TICKTICK_REDIRECT_URI")
            .unwrap_or_else(|| DEFAULT_REDIRECT_URI.to_string());
        let broker_url = optional_env(&get_var, "TICKTICK_OAUTH_BROKER_URL");
        let broker_api_key = optional_env(&get_var, "TICKTICK_OAUTH_BROKER_KEY");

        let client_secret = if broker_url.is_some() {
            optional_env(&get_var, "TICKTICK_CLIENT_SECRET")
        } else {
            Some(required_env(&get_var, "TICKTICK_CLIENT_SECRET")?)
        };

        Ok(Self {
            client_id,
            client_secret,
            redirect_uri,
            broker_url,
            broker_api_key,
        })
    }

    pub fn oauth_client(&self) -> Result<TickTickOAuth> {
        TickTickOAuth::new(
            self.client_id.clone(),
            self.client_secret.clone(),
            self.redirect_uri.clone(),
        )
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn uses_broker(&self) -> bool {
        self.broker_url.is_some()
    }

    pub async fn exchange_code(
        &self,
        code: AuthorizationCode,
        pkce_verifier: PkceCodeVerifier,
    ) -> Result<TokenResponseData> {
        match self.broker_url.as_deref() {
            Some(broker_url) => {
                TickTickOAuth::exchange_code_via_broker(
                    code,
                    pkce_verifier,
                    self.redirect_uri.clone(),
                    broker_url,
                    self.broker_api_key.as_deref(),
                )
                .await
            }
            None => {
                self.oauth_client()?
                    .exchange_code(code, pkce_verifier)
                    .await
            }
        }
    }

    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenResponseData> {
        let refresh_token = refresh_token.trim();
        if refresh_token.is_empty() {
            return Err(anyhow!("Missing refresh token in saved config"));
        }

        match self.broker_url.as_deref() {
            Some(broker_url) => {
                TickTickOAuth::refresh_token_via_broker(
                    refresh_token,
                    broker_url,
                    self.broker_api_key.as_deref(),
                )
                .await
            }
            None => self.oauth_client()?.refresh_token(refresh_token).await,
        }
    }
}

pub async fn refresh_config_if_needed(app_config: &AppConfig, config: Config) -> Result<Config> {
    if !token_needs_refresh(config.expires_at)? {
        return Ok(config);
    }

    let settings = AuthSettings::from_env()?;
    refresh_config_if_needed_with_settings(app_config, config, &settings).await
}

async fn refresh_config_if_needed_with_settings(
    app_config: &AppConfig,
    config: Config,
    settings: &AuthSettings,
) -> Result<Config> {
    refresh_config_if_needed_with_action(app_config, config, settings, |settings, refresh_token| {
        let settings = settings.clone();
        let refresh_token = refresh_token.to_string();
        async move { settings.refresh_token(&refresh_token).await }
    })
    .await
}

async fn refresh_config_if_needed_with_action<F, Fut>(
    app_config: &AppConfig,
    config: Config,
    settings: &AuthSettings,
    refresh_action: F,
) -> Result<Config>
where
    F: FnOnce(&AuthSettings, &str) -> Fut,
    Fut: Future<Output = Result<TokenResponseData>>,
{
    if !token_needs_refresh(config.expires_at)? {
        return Ok(config);
    }

    let existing_refresh_token = config.refresh_token.clone();
    let token = refresh_action(settings, &existing_refresh_token).await?;
    let refreshed = Config {
        access_token: token.access_token,
        refresh_token: if token.refresh_token.is_empty() {
            config.refresh_token
        } else {
            token.refresh_token
        },
        expires_at: token.expires_at,
    };

    app_config.save(&refreshed)?;
    Ok(refreshed)
}

fn required_env<F>(get_var: &F, key: &str) -> Result<String>
where
    F: Fn(&str) -> std::result::Result<String, std::env::VarError>,
{
    optional_env(get_var, key).ok_or_else(|| anyhow!("Missing {}", key))
}

fn optional_env<F>(get_var: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> std::result::Result<String, std::env::VarError>,
{
    get_var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn token_needs_refresh(expires_at: i64) -> Result<bool> {
    Ok(expires_at <= unix_timestamp()? + REFRESH_MARGIN_SECS)
}

fn unix_timestamp() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System time is before UNIX_EPOCH")?
        .as_secs() as i64)
}

#[derive(Debug, Clone)]
pub struct TickTickOAuth {
    client: BasicClient,
    client_id: String,
    client_secret: Option<String>,
    scopes: Vec<String>,
    token_url: String,
}

impl TickTickOAuth {
    pub fn new(
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
    ) -> Result<Self> {
        Self::new_with_urls(
            client_id,
            client_secret,
            redirect_uri,
            DEFAULT_AUTH_URL,
            DEFAULT_TOKEN_URL,
        )
    }

    fn new_with_urls(
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
        auth_url: &str,
        token_url: &str,
    ) -> Result<Self> {
        let auth_url = AuthUrl::new(auth_url.to_string())?;
        let token_url = token_url.to_string();
        let token_url_value = TokenUrl::new(token_url.clone())?;
        let redirect_url = RedirectUrl::new(redirect_uri)?;

        let client = BasicClient::new(
            ClientId::new(client_id.clone()),
            client_secret.clone().map(ClientSecret::new),
            auth_url,
            Some(token_url_value),
        )
        .set_redirect_uri(redirect_url);

        Ok(Self {
            client,
            client_id,
            client_secret,
            scopes: vec!["tasks:write".to_string(), "tasks:read".to_string()],
            token_url: token_url.to_string(),
        })
    }

    pub fn auth_url(&self) -> (String, PkceCodeVerifier, CsrfToken) {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let scopes: Vec<Scope> = self.scopes.iter().cloned().map(Scope::new).collect();

        let (auth_url, csrf_token) = self
            .client
            .authorize_url(CsrfToken::new_random)
            .add_scopes(scopes)
            .set_pkce_challenge(pkce_challenge)
            .url();

        (auth_url.to_string(), pkce_verifier, csrf_token)
    }

    pub async fn exchange_code(
        &self,
        code: AuthorizationCode,
        pkce_verifier: PkceCodeVerifier,
    ) -> Result<TokenResponseData> {
        let token = self
            .client
            .exchange_code(code)
            .set_pkce_verifier(pkce_verifier)
            .request_async(async_http_client)
            .await?;

        Ok(TokenResponseData::from_oauth_token_response(
            token.access_token().secret().to_string(),
            token.refresh_token().map(|t| t.secret().to_string()),
            token.expires_in().map(|duration| duration.as_secs() as i64),
        )?)
    }

    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenResponseData> {
        let request = build_direct_refresh_request(
            &self.client_id,
            self.client_secret.as_deref(),
            &self.token_url,
            refresh_token,
        )?;
        let client = reqwest::Client::new();
        let response = client
            .execute(build_request(&client, request)?)
            .await
            .context("Failed to refresh OAuth token")?;

        let status = response.status();
        if !status.is_success() {
            let details = response
                .text()
                .await
                .unwrap_or_else(|_| "No response body".to_string());
            return Err(anyhow!(
                "TickTick token endpoint returned {}: {}",
                status.as_u16(),
                details
            ));
        }

        let token = response
            .json::<TokenEndpointResponse>()
            .await
            .context("Failed to parse TickTick token refresh response")?;

        TokenResponseData::from_token_endpoint_response(token)
    }

    pub async fn exchange_code_via_broker(
        code: AuthorizationCode,
        pkce_verifier: PkceCodeVerifier,
        redirect_uri: String,
        broker_url: &str,
        broker_api_key: Option<&str>,
    ) -> Result<TokenResponseData> {
        let payload = BrokerExchangeRequest {
            code: code.secret().to_string(),
            code_verifier: pkce_verifier.secret().to_string(),
            redirect_uri,
        };

        Self::send_broker_token_request(broker_url, "/v1/oauth/exchange", &payload, broker_api_key)
            .await
    }

    pub async fn refresh_token_via_broker(
        refresh_token: &str,
        broker_url: &str,
        broker_api_key: Option<&str>,
    ) -> Result<TokenResponseData> {
        let payload = BrokerRefreshRequest {
            refresh_token: refresh_token.to_string(),
        };

        Self::send_broker_token_request(broker_url, "/v1/oauth/refresh", &payload, broker_api_key)
            .await
    }

    async fn send_broker_token_request<T: Serialize>(
        broker_url: &str,
        path: &str,
        payload: &T,
        broker_api_key: Option<&str>,
    ) -> Result<TokenResponseData> {
        let client = reqwest::Client::new();
        let request = build_broker_request(broker_url, path, payload, broker_api_key)?;
        let response = client
            .execute(build_request(&client, request)?)
            .await
            .context("Failed to call OAuth broker")?;

        let status = response.status();
        if !status.is_success() {
            let details = response
                .text()
                .await
                .unwrap_or_else(|_| "No response body".to_string());
            return Err(anyhow!(
                "OAuth broker returned {}: {}",
                status.as_u16(),
                details
            ));
        }

        let token = response
            .json::<TokenEndpointResponse>()
            .await
            .context("Failed to parse OAuth broker token response")?;

        TokenResponseData::from_token_endpoint_response(token)
    }
}

fn build_direct_refresh_request(
    client_id: &str,
    client_secret: Option<&str>,
    token_url: &str,
    refresh_token: &str,
) -> Result<PreparedRequest> {
    let client_secret = client_secret
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Missing TICKTICK_CLIENT_SECRET"))?;
    let refresh_token = refresh_token.trim();
    if refresh_token.is_empty() {
        return Err(anyhow!("Missing refresh token in saved config"));
    }
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("grant_type", "refresh_token");
    serializer.append_pair("refresh_token", refresh_token);

    Ok(PreparedRequest {
        url: token_url.to_string(),
        basic_auth: Some((client_id.to_string(), client_secret.to_string())),
        broker_api_key: None,
        content_type: "application/x-www-form-urlencoded",
        body: serializer.finish().into_bytes(),
    })
}

fn build_broker_request<T: Serialize>(
    broker_url: &str,
    path: &str,
    payload: &T,
    broker_api_key: Option<&str>,
) -> Result<PreparedRequest> {
    Ok(PreparedRequest {
        url: format!("{}{}", broker_url.trim_end_matches('/'), path),
        basic_auth: None,
        broker_api_key: broker_api_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        content_type: "application/json",
        body: serde_json::to_vec(payload).context("Failed to serialize OAuth broker payload")?,
    })
}

fn build_request(client: &reqwest::Client, request: PreparedRequest) -> Result<reqwest::Request> {
    let mut builder = client
        .post(&request.url)
        .header("content-type", request.content_type)
        .body(request.body);

    if let Some((client_id, client_secret)) = request.basic_auth {
        builder = builder.basic_auth(client_id, Some(client_secret));
    }
    if let Some(broker_api_key) = request.broker_api_key {
        builder = builder.header("x-broker-key", broker_api_key);
    }

    builder.build().context("Failed to build HTTP request")
}

#[derive(Debug, Serialize)]
struct BrokerExchangeRequest {
    code: String,
    code_verifier: String,
    redirect_uri: String,
}

#[derive(Debug, Serialize)]
struct BrokerRefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenEndpointResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
struct PreparedRequest {
    url: String,
    basic_auth: Option<(String, String)>,
    broker_api_key: Option<String>,
    content_type: &'static str,
    body: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponseData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

impl TokenResponseData {
    fn from_oauth_token_response(
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
    ) -> Result<Self> {
        Ok(Self {
            access_token,
            refresh_token: refresh_token.unwrap_or_default(),
            expires_at: unix_timestamp()? + expires_in.unwrap_or(DEFAULT_EXPIRES_IN_SECS),
        })
    }

    fn from_token_endpoint_response(token: TokenEndpointResponse) -> Result<Self> {
        Ok(Self {
            access_token: token.access_token,
            refresh_token: token.refresh_token.unwrap_or_default(),
            expires_at: unix_timestamp()? + token.expires_in.unwrap_or(DEFAULT_EXPIRES_IN_SECS),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn auth_settings_allows_broker_without_client_secret() {
        let values = HashMap::from([
            ("TICKTICK_CLIENT_ID", "client-id"),
            ("TICKTICK_OAUTH_BROKER_URL", "https://broker.example"),
            ("TICKTICK_OAUTH_BROKER_KEY", "secret-key"),
        ]);
        let settings = AuthSettings::from_env_with(|key| {
            values
                .get(key)
                .map(|value| value.to_string())
                .ok_or(std::env::VarError::NotPresent)
        })
        .unwrap();

        assert_eq!(settings.client_id, "client-id");
        assert_eq!(settings.client_secret, None);
        assert_eq!(settings.redirect_uri, DEFAULT_REDIRECT_URI);
        assert_eq!(
            settings.broker_url.as_deref(),
            Some("https://broker.example")
        );
        assert_eq!(settings.broker_api_key.as_deref(), Some("secret-key"));
    }

    #[test]
    fn direct_refresh_builds_basic_auth_form_request() {
        let request = build_direct_refresh_request(
            "client-id",
            Some("client-secret"),
            "https://ticktick.example/oauth/token",
            "refresh-me",
        )
        .unwrap();

        assert_eq!(request.url, "https://ticktick.example/oauth/token");
        assert_eq!(
            request.basic_auth,
            Some(("client-id".to_string(), "client-secret".to_string()))
        );
        assert_eq!(request.content_type, "application/x-www-form-urlencoded");
        assert_eq!(
            String::from_utf8(request.body).unwrap(),
            "grant_type=refresh_token&refresh_token=refresh-me"
        );
    }

    #[test]
    fn broker_refresh_builds_json_request_with_optional_key() {
        let payload = BrokerRefreshRequest {
            refresh_token: "broker-refresh".to_string(),
        };
        let request = build_broker_request(
            "https://broker.example/",
            "/v1/oauth/refresh",
            &payload,
            Some("broker-key"),
        )
        .unwrap();

        assert_eq!(request.url, "https://broker.example/v1/oauth/refresh");
        assert_eq!(request.basic_auth, None);
        assert_eq!(request.broker_api_key.as_deref(), Some("broker-key"));
        assert_eq!(request.content_type, "application/json");
        assert_eq!(
            String::from_utf8(request.body).unwrap(),
            r#"{"refresh_token":"broker-refresh"}"#
        );
    }

    #[tokio::test]
    async fn refresh_config_if_needed_refreshes_expired_config_and_saves_it() {
        let temp_dir = unique_temp_dir("auth-refresh");
        fs::create_dir_all(&temp_dir).unwrap();
        let app_config = AppConfig {
            config_file: temp_dir.join("config.toml"),
        };
        let config = Config {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            expires_at: 0,
        };
        let settings = AuthSettings {
            client_id: "client-id".to_string(),
            client_secret: Some("client-secret".to_string()),
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            broker_url: None,
            broker_api_key: None,
        };

        let refreshed =
            refresh_config_if_needed_with_action(&app_config, config, &settings, |_, _| async {
                Ok(TokenResponseData {
                    access_token: "updated-access".to_string(),
                    refresh_token: "updated-refresh".to_string(),
                    expires_at: unix_timestamp().unwrap() + 600,
                })
            })
            .await
            .unwrap();
        let saved = app_config.load().unwrap().unwrap();

        assert_eq!(refreshed, saved);
        assert_eq!(saved.access_token, "updated-access");
        assert_eq!(saved.refresh_token, "updated-refresh");

        let _ = fs::remove_dir_all(temp_dir);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), now))
    }
}
