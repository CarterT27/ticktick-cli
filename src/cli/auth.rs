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

const DEFAULT_REDIRECT_URI: &str = "http://localhost:8080/callback";

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
    let redirect_uri =
        std::env::var("TICKTICK_REDIRECT_URI").unwrap_or_else(|_| DEFAULT_REDIRECT_URI.to_string());

    let broker_url = std::env::var("TICKTICK_OAUTH_BROKER_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let broker_api_key = std::env::var("TICKTICK_OAUTH_BROKER_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let client_secret = if broker_url.is_none() {
        Some(
            std::env::var("TICKTICK_CLIENT_SECRET")
                .map_err(|_| anyhow!("Missing TICKTICK_CLIENT_SECRET"))?,
        )
    } else {
        std::env::var("TICKTICK_CLIENT_SECRET").ok()
    };

    if broker_url.is_some() {
        println!("Using OAuth broker for token exchange.");
    }

    let oauth = TickTickOAuth::new(client_id, client_secret, redirect_uri.clone())?;
    let (auth_url, pkce_verifier, csrf_token) = oauth.auth_url();

    println!("Opening browser for authorization...");
    if webbrowser::open(&auth_url).is_err() {
        println!("Open this URL in your browser:");
        println!("{}", auth_url);
    }

    let code = wait_for_code(&redirect_uri, csrf_token)?;
    let token = match broker_url {
        Some(url) => {
            TickTickOAuth::exchange_code_via_broker(
                AuthorizationCode::new(code),
                pkce_verifier,
                redirect_uri,
                &url,
                broker_api_key.as_deref(),
            )
            .await?
        }
        None => {
            oauth
                .exchange_code(AuthorizationCode::new(code), pkce_verifier)
                .await?
        }
    };

    let config = Config {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: token.expires_at,
    };

    let app_config = AppConfig::new()?;
    app_config.save(&config)?;

    println!("Successfully authenticated!");
    println!(
        "Credentials stored in {}",
        app_config.config_file_path().display()
    );
    Ok(())
}

fn wait_for_code(redirect_uri: &str, csrf_token: CsrfToken) -> Result<String> {
    let callback_url = Url::parse(redirect_uri)
        .map_err(|err| anyhow!("Invalid TICKTICK_REDIRECT_URI '{}': {}", redirect_uri, err))?;

    let port = callback_url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("Redirect URI must include a valid port"))?;

    let bind_addr = format!("127.0.0.1:{}", port);
    let server =
        Server::http(&bind_addr).map_err(|err| anyhow!("Failed to start local server: {}", err))?;

    let expected_path = callback_url.path().to_string();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Ok(request) = server.recv() {
            let url = format!("http://localhost{}", request.url());
            let parsed = Url::parse(&url).ok();
            let mut code: Option<String> = None;
            let mut state: Option<String> = None;
            let mut path_matches = false;

            if let Some(parsed) = parsed {
                path_matches = parsed.path() == expected_path;
                for (key, value) in parsed.query_pairs() {
                    if key == "code" {
                        code = Some(value.to_string());
                    }
                    if key == "state" {
                        state = Some(value.to_string());
                    }
                }
            }

            let body = if path_matches {
                "Authentication complete. You can close this window."
            } else {
                "Unexpected callback path. You can close this window and try again."
            };
            let _ = request.respond(Response::from_string(body));
            if path_matches {
                let _ = tx.send((code, state));
            } else {
                let _ = tx.send((None, None));
            }
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
    println!("Successfully logged out.");
    Ok(())
}

pub async fn status() -> Result<()> {
    let app_config = AppConfig::new()?;

    match app_config.load()? {
        Some(config) => {
            println!("Status: Authenticated");
            println!(
                "Access Token: {}...{}",
                &config.access_token[0..8],
                &config.access_token[config.access_token.len() - 8..]
            );

            let now = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)?
                .as_secs() as i64;

            let remaining = config.expires_at - now;

            if remaining > 0 {
                println!("Token expires in: {} minutes", remaining / 60);
            } else {
                println!("Token expired! Please login again.");
            }
        }
        None => {
            println!("Status: Not authenticated");
            println!("Run 'tt auth login' to authenticate.");
        }
    }

    Ok(())
}
