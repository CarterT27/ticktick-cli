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
    Login,
    Logout,
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
            let parsed = Url::parse(&url).ok();
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
