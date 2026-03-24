use crate::cache::CacheStore;
use crate::config::auth::TickTickOAuth;
use crate::config::AppConfig;
use crate::config::Config;
use anyhow::{anyhow, Result};
use clap::Subcommand;
use oauth2::{AuthorizationCode, CsrfToken};
use std::sync::mpsc;
use std::time::Duration;
use tiny_http::{Response, Server};
use url::{Host, Url};

const DEFAULT_REDIRECT_URI: &str = "http://localhost:8080/callback";

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalCallbackConfig {
    bind_addr: String,
    callback_origin: String,
    callback_path: String,
}

#[derive(Subcommand)]
pub enum AuthCommands {
    #[command(alias = "signin")]
    Login,
    #[command(alias = "signout")]
    Logout,
    #[command(alias = "whoami")]
    Status,
}

pub async fn login() -> Result<()> {
    println!("TickTick CLI Authentication");
    println!("=========================");
    println!();

    let client_id =
        std::env::var("TICKTICK_CLIENT_ID").map_err(|_| anyhow!("Missing TICKTICK_CLIENT_ID"))?;
    let client_secret = std::env::var("TICKTICK_CLIENT_SECRET")
        .map_err(|_| anyhow!("Missing TICKTICK_CLIENT_SECRET"))?;
    let redirect_uri =
        std::env::var("TICKTICK_REDIRECT_URI").unwrap_or_else(|_| DEFAULT_REDIRECT_URI.to_string());
    let callback_config = LocalCallbackConfig::from_redirect_uri(&redirect_uri)?;

    let oauth = TickTickOAuth::new(client_id, client_secret, redirect_uri)?;
    let (auth_url, pkce_verifier, csrf_token) = oauth.auth_url();

    println!("Opening browser for authorization...");
    if webbrowser::open(&auth_url).is_err() {
        println!("Open this URL in your browser:");
        println!("{}", auth_url);
    }

    let code = wait_for_code(csrf_token, callback_config)?;
    let token = oauth
        .exchange_code(AuthorizationCode::new(code), pkce_verifier)
        .await?;

    let config = Config {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: token.expires_at,
    };

    let app_config = AppConfig::new()?;
    app_config.save(&config)?;
    if let Ok(cache) = CacheStore::new() {
        let _ = cache.clear_all();
    }

    println!("Successfully authenticated!");
    println!(
        "Credentials stored in {}",
        app_config.config_file_path().display()
    );
    Ok(())
}

fn wait_for_code(csrf_token: CsrfToken, callback_config: LocalCallbackConfig) -> Result<String> {
    let server = Server::http(&callback_config.bind_addr)
        .map_err(|err| anyhow!("Failed to start local server: {}", err))?;
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        while let Ok(request) = server.recv() {
            let Some(callback_url) = callback_config.callback_url_for_request_target(request.url())
            else {
                let _ = request.respond(
                    Response::from_string("Unexpected OAuth callback path.").with_status_code(404),
                );
                continue;
            };

            let (code, state) = extract_callback_params(&callback_url);
            let body = "Authentication complete. You can close this window.";
            let _ = request.respond(Response::from_string(body));
            let _ = tx.send((code, state));
            break;
        }
    });

    let (code, state) = rx
        .recv_timeout(Duration::from_secs(120))
        .map_err(|_| anyhow!("Timed out waiting for OAuth callback"))?;

    let state = state.ok_or_else(|| anyhow!("Missing state parameter"))?;
    if state != csrf_token.secret().as_str() {
        return Err(anyhow!("Invalid OAuth state"));
    }

    code.ok_or_else(|| anyhow!("Missing authorization code"))
}

pub async fn logout() -> Result<()> {
    let app_config = AppConfig::new()?;
    app_config.clear()?;
    if let Ok(cache) = CacheStore::new() {
        let _ = cache.clear_all();
    }
    println!("Successfully logged out.");
    Ok(())
}

pub async fn status() -> Result<()> {
    let app_config = AppConfig::new()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_secs() as i64;

    for line in format_status_lines(app_config.load()?.as_ref(), now) {
        println!("{}", line);
    }

    Ok(())
}

fn extract_callback_params(url: &str) -> (Option<String>, Option<String>) {
    let parsed = Url::parse(url).ok();
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;

    if let Some(parsed) = parsed {
        for (key, value) in parsed.query_pairs() {
            if key == "code" {
                code = Some(value.to_string());
            }
            if key == "state" {
                state = Some(value.to_string());
            }
        }
    }

    (code, state)
}

impl LocalCallbackConfig {
    fn from_redirect_uri(redirect_uri: &str) -> Result<Self> {
        let parsed = Url::parse(redirect_uri)?;
        if parsed.scheme() != "http" {
            return Err(anyhow!(
                "TICKTICK_REDIRECT_URI must use http for the local callback server"
            ));
        }

        let host = parsed
            .host()
            .ok_or_else(|| anyhow!("TICKTICK_REDIRECT_URI must include a host"))?;
        if !is_loopback_host(&host) {
            return Err(anyhow!(
                "TICKTICK_REDIRECT_URI must use a loopback host such as localhost, 127.0.0.1, or ::1"
            ));
        }

        let port = parsed
            .port()
            .ok_or_else(|| anyhow!("TICKTICK_REDIRECT_URI must include an explicit port"))?;
        let host = format_host(&host);
        let path = normalize_callback_path(parsed.path());

        Ok(Self {
            bind_addr: format!("{}:{}", host, port),
            callback_origin: format!("http://{}:{}", host, port),
            callback_path: path,
        })
    }

    fn callback_url_for_request_target(&self, request_target: &str) -> Option<String> {
        let request_path = request_target.split('?').next().unwrap_or_default();
        if request_path != self.callback_path {
            return None;
        }

        Some(format!("{}{}", self.callback_origin, request_target))
    }
}

fn is_loopback_host(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => *domain == "localhost",
        Host::Ipv4(addr) => addr.is_loopback(),
        Host::Ipv6(addr) => addr.is_loopback(),
    }
}

fn format_host(host: &Host<&str>) -> String {
    match host {
        Host::Domain(domain) => (*domain).to_string(),
        Host::Ipv4(addr) => addr.to_string(),
        Host::Ipv6(addr) => format!("[{}]", addr),
    }
}

fn normalize_callback_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

fn format_status_lines(config: Option<&Config>, now: i64) -> Vec<String> {
    match config {
        Some(config) => {
            let remaining = config.expires_at - now;
            let mut lines = vec![
                "Status: Authenticated".to_string(),
                format!(
                    "Access Token: {}...{}",
                    &config.access_token[0..8],
                    &config.access_token[config.access_token.len() - 8..]
                ),
            ];

            if remaining > 0 {
                lines.push(format!("Token expires in: {} minutes", remaining / 60));
            } else {
                lines.push("Token expired! Please login again.".to_string());
            }

            lines
        }
        None => vec![
            "Status: Not authenticated".to_string(),
            "Run 'tt auth login' to authenticate.".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(expires_at: i64) -> Config {
        Config {
            access_token: "12345678abcdefgh".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at,
        }
    }

    #[test]
    fn extract_callback_params_returns_code_and_state() {
        let (code, state) =
            extract_callback_params("http://localhost/callback?code=auth-code&state=csrf-token");

        assert_eq!(code.as_deref(), Some("auth-code"));
        assert_eq!(state.as_deref(), Some("csrf-token"));
    }

    #[test]
    fn extract_callback_params_handles_invalid_urls() {
        let (code, state) = extract_callback_params("not a url");
        assert_eq!(code, None);
        assert_eq!(state, None);
    }

    #[test]
    fn local_callback_config_uses_redirect_uri_host_port_and_path() {
        let callback =
            LocalCallbackConfig::from_redirect_uri("http://127.0.0.1:9090/custom/callback")
                .unwrap();

        assert_eq!(callback.bind_addr, "127.0.0.1:9090");
        assert_eq!(callback.callback_origin, "http://127.0.0.1:9090");
        assert_eq!(callback.callback_path, "/custom/callback");
    }

    #[test]
    fn local_callback_config_rejects_non_loopback_redirect_hosts() {
        let error = LocalCallbackConfig::from_redirect_uri("http://example.com:8080/callback")
            .unwrap_err()
            .to_string();

        assert!(error.contains("loopback host"));
    }

    #[test]
    fn local_callback_config_requires_an_explicit_port() {
        let error = LocalCallbackConfig::from_redirect_uri("http://localhost/callback")
            .unwrap_err()
            .to_string();

        assert!(error.contains("explicit port"));
    }

    #[test]
    fn callback_url_for_request_target_requires_matching_path() {
        let callback =
            LocalCallbackConfig::from_redirect_uri("http://localhost:8080/callback").unwrap();

        assert_eq!(
            callback.callback_url_for_request_target("/callback?code=a&state=b"),
            Some("http://localhost:8080/callback?code=a&state=b".to_string())
        );
        assert_eq!(
            callback.callback_url_for_request_target("/favicon.ico"),
            None
        );
    }

    #[test]
    fn format_status_lines_for_authenticated_session() {
        let lines = format_status_lines(Some(&sample_config(4_000)), 1_000);

        assert_eq!(lines[0], "Status: Authenticated");
        assert_eq!(lines[1], "Access Token: 12345678...abcdefgh");
        assert_eq!(lines[2], "Token expires in: 50 minutes");
    }

    #[test]
    fn format_status_lines_for_expired_or_missing_session() {
        let expired = format_status_lines(Some(&sample_config(900)), 1_000);
        assert_eq!(expired[2], "Token expired! Please login again.");

        let missing = format_status_lines(None, 1_000);
        assert_eq!(missing[0], "Status: Not authenticated");
        assert_eq!(missing[1], "Run 'tt auth login' to authenticate.");
    }
}
