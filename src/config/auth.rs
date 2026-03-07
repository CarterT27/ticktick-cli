use super::{AppConfig, Config};
use anyhow::{anyhow, Context, Result};
use oauth2::{
    basic::BasicClient, reqwest::async_http_client, AuthUrl, AuthorizationCode, ClientId,
    ClientSecret, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken, Scope,
    TokenResponse, TokenType, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

    pub async fn refresh_access_token(&self, refresh_token: &str) -> Result<TokenResponseData> {
        let refresh_token = refresh_token.trim();
        if refresh_token.is_empty() {
            return Err(anyhow!("Missing refresh token in saved config"));
        }

        match self.broker_url.as_deref() {
            Some(broker_url) => {
                TickTickOAuth::refresh_access_token_via_broker(
                    refresh_token,
                    broker_url,
                    self.broker_api_key.as_deref(),
                )
                .await
            }
            None => {
                self.oauth_client()?
                    .refresh_access_token(refresh_token)
                    .await
            }
        }
    }
}

pub async fn refresh_config_if_needed(app_config: &AppConfig, config: Config) -> Result<Config> {
    if !token_needs_refresh(config.expires_at)? {
        return Ok(config);
    }

    let settings = AuthSettings::from_env()?;
    let current_refresh_token = config.refresh_token.clone();
    let token = settings.refresh_access_token(&current_refresh_token).await?;

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
    scopes: Vec<String>,
}

impl TickTickOAuth {
    pub fn new(
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
    ) -> Result<Self> {
        let auth_url = AuthUrl::new(DEFAULT_AUTH_URL.to_string())?;
        let token_url = TokenUrl::new(DEFAULT_TOKEN_URL.to_string())?;
        let redirect_url = RedirectUrl::new(redirect_uri)?;

        let client = BasicClient::new(
            ClientId::new(client_id),
            client_secret.map(ClientSecret::new),
            auth_url,
            Some(token_url),
        )
        .set_redirect_uri(redirect_url);

        Ok(Self {
            client,
            scopes: vec!["tasks:write".to_string(), "tasks:read".to_string()],
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

        token_response_data(&token)
    }

    pub async fn refresh_access_token(&self, refresh_token: &str) -> Result<TokenResponseData> {
        let token = self
            .client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
            .request_async(async_http_client)
            .await?;

        token_response_data(&token)
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

        send_broker_token_request(
            broker_url,
            "/v1/oauth/exchange",
            &payload,
            broker_api_key,
        )
        .await
    }

    pub async fn refresh_access_token_via_broker(
        refresh_token: &str,
        broker_url: &str,
        broker_api_key: Option<&str>,
    ) -> Result<TokenResponseData> {
        let payload = BrokerRefreshRequest {
            refresh_token: refresh_token.to_string(),
        };

        send_broker_token_request(
            broker_url,
            "/v1/oauth/refresh",
            &payload,
            broker_api_key,
        )
        .await
    }
}

async fn send_broker_token_request<T: Serialize>(
    broker_url: &str,
    path: &str,
    payload: &T,
    broker_api_key: Option<&str>,
) -> Result<TokenResponseData> {
    let endpoint = format!("{}{}", broker_url.trim_end_matches('/'), path);
    let client = reqwest::Client::new();
    let mut request = client.post(endpoint).json(payload);
    if let Some(key) = broker_api_key.filter(|value| !value.trim().is_empty()) {
        request = request.header("x-broker-key", key);
    }

    let response = request
        .send()
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponseData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

impl TokenResponseData {
    fn from_token_endpoint_response(token: TokenEndpointResponse) -> Result<Self> {
        Ok(Self {
            access_token: token.access_token,
            refresh_token: token.refresh_token.unwrap_or_default(),
            expires_at: unix_timestamp()? + token.expires_in.unwrap_or(DEFAULT_EXPIRES_IN_SECS),
        })
    }
}

fn token_response_data<T, TT>(token: &T) -> Result<TokenResponseData>
where
    T: TokenResponse<TT>,
    TT: TokenType,
{
    Ok(TokenResponseData {
        access_token: token.access_token().secret().to_string(),
        refresh_token: token
            .refresh_token()
            .map(|token| token.secret().to_string())
            .unwrap_or_default(),
        expires_at: unix_timestamp()?
            + token
                .expires_in()
                .unwrap_or(Duration::from_secs(DEFAULT_EXPIRES_IN_SECS as u64))
                .as_secs() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use url::Url;

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
    fn new_rejects_invalid_redirect_uri() {
        let result = TickTickOAuth::new(
            "client-id".to_string(),
            Some("client-secret".to_string()),
            "not a valid redirect".to_string(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn auth_url_contains_expected_oauth_parameters() {
        let oauth = TickTickOAuth::new(
            "client-id".to_string(),
            Some("client-secret".to_string()),
            "http://localhost/callback".to_string(),
        )
        .unwrap();

        let (auth_url, pkce_verifier, csrf_token) = oauth.auth_url();
        let parsed = Url::parse(&auth_url).unwrap();
        let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("ticktick.com"));
        assert_eq!(parsed.path(), "/oauth/authorize");
        assert_eq!(query.get("client_id"), Some(&"client-id".to_string()));
        assert_eq!(
            query.get("redirect_uri"),
            Some(&"http://localhost/callback".to_string())
        );
        assert_eq!(query.get("response_type"), Some(&"code".to_string()));
        assert_eq!(
            query.get("scope"),
            Some(&"tasks:write tasks:read".to_string())
        );
        assert_eq!(
            query.get("code_challenge_method"),
            Some(&"S256".to_string())
        );
        assert!(query.contains_key("code_challenge"));
        assert!(!pkce_verifier.secret().is_empty());
        assert!(!csrf_token.secret().is_empty());
    }
}
