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
use url::Url;

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
    let redirect_uri = std::env::var("TICKTICK_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:8080/callback".to_string());

    let oauth = TickTickOAuth::new(client_id, client_secret, redirect_uri)?;
    let (auth_url, pkce_verifier, csrf_token) = oauth.auth_url();

    println!("Opening browser for authorization...");
    if webbrowser::open(&auth_url).is_err() {
        println!("Open this URL in your browser:");
        println!("{}", auth_url);
    }

    let code = wait_for_code(csrf_token)?;
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

fn wait_for_code(csrf_token: CsrfToken) -> Result<String> {
    let server = Server::http("127.0.0.1:8080")
        .map_err(|err| anyhow!("Failed to start local server: {}", err))?;
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Ok(request) = server.recv() {
            let url = format!("http://localhost{}", request.url());
            let (code, state) = extract_callback_params(&url);

            let body = "Authentication complete. You can close this window.";
            let _ = request.respond(Response::from_string(body));
            let _ = tx.send((code, state));
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
