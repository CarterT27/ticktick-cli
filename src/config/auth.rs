use anyhow::{anyhow, Context, Result};
use oauth2::{
    basic::BasicClient, reqwest::async_http_client, AuthUrl, AuthorizationCode, ClientId,
    ClientSecret, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

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
        let auth_url = AuthUrl::new("https://ticktick.com/oauth/authorize".to_string())?;
        let token_url = TokenUrl::new("https://ticktick.com/oauth/token".to_string())?;
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

        let expires_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs() as i64
            + token
                .expires_in()
                .unwrap_or(Duration::from_secs(3600))
                .as_secs() as i64;

        Ok(TokenResponseData {
            access_token: token.access_token().secret().to_string(),
            refresh_token: token
                .refresh_token()
                .map(|t| t.secret().to_string())
                .unwrap_or_default(),
            expires_at,
        })
    }

    pub async fn exchange_code_via_broker(
        code: AuthorizationCode,
        pkce_verifier: PkceCodeVerifier,
        redirect_uri: String,
        broker_url: &str,
        broker_api_key: Option<&str>,
    ) -> Result<TokenResponseData> {
        let endpoint = format!("{}/v1/oauth/exchange", broker_url.trim_end_matches('/'));
        let payload = BrokerExchangeRequest {
            code: code.secret().to_string(),
            code_verifier: pkce_verifier.secret().to_string(),
            redirect_uri,
        };

        let client = reqwest::Client::new();
        let mut request = client.post(endpoint).json(&payload);
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
            .json::<BrokerTokenResponse>()
            .await
            .context("Failed to parse OAuth broker token response")?;

        let expires_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs() as i64
            + token.expires_in.unwrap_or(3600);

        Ok(TokenResponseData {
            access_token: token.access_token,
            refresh_token: token.refresh_token.unwrap_or_default(),
            expires_at,
        })
    }
}

#[derive(Debug, Serialize)]
struct BrokerExchangeRequest {
    code: String,
    code_verifier: String,
    redirect_uri: String,
}

#[derive(Debug, Deserialize)]
struct BrokerTokenResponse {
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
