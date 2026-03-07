use anyhow::Result;
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
    pub fn new(client_id: String, client_secret: String, redirect_uri: String) -> Result<Self> {
        let auth_url = AuthUrl::new("https://ticktick.com/oauth/authorize".to_string())?;
        let token_url = TokenUrl::new("https://ticktick.com/oauth/token".to_string())?;
        let redirect_url = RedirectUrl::new(redirect_uri)?;

        let client = BasicClient::new(
            ClientId::new(client_id),
            Some(ClientSecret::new(client_secret)),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponseData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use url::Url;

    #[test]
    fn new_rejects_invalid_redirect_uri() {
        let result = TickTickOAuth::new(
            "client-id".to_string(),
            "client-secret".to_string(),
            "not a valid redirect".to_string(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn auth_url_contains_expected_oauth_parameters() {
        let oauth = TickTickOAuth::new(
            "client-id".to_string(),
            "client-secret".to_string(),
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
