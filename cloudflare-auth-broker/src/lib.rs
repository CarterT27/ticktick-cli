use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use worker::*;

const TICKTICK_TOKEN_URL: &str = "https://ticktick.com/oauth/token";

#[derive(Debug, Deserialize)]
struct ExchangeRequest {
    code: String,
    code_verifier: String,
    redirect_uri: String,
}

#[derive(Debug, Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct TickTickTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    token_type: String,
    #[serde(default)]
    scope: String,
    expires_in: Option<i64>,
}

#[event(fetch, respond_with_errors)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .get_async("/health", |_req, _ctx| async move { Response::ok("ok") })
        .post_async("/v1/oauth/exchange", |mut req, ctx| async move {
            if let Some(response) = authorize_request(&req, &ctx)? {
                return Ok(response);
            }

            let payload = match req.json::<ExchangeRequest>().await {
                Ok(payload) => payload,
                Err(_) => return Response::error("Invalid JSON body", 400),
            };

            if is_blank(&payload.code)
                || is_blank(&payload.code_verifier)
                || is_blank(&payload.redirect_uri)
            {
                return Response::error("Missing code, code_verifier, or redirect_uri", 400);
            }

            let body = format!(
                "grant_type=authorization_code&code={}&redirect_uri={}&code_verifier={}",
                urlencoding::encode(payload.code.trim()),
                urlencoding::encode(payload.redirect_uri.trim()),
                urlencoding::encode(payload.code_verifier.trim())
            );

            let token = exchange_token(&ctx, body).await?;
            let mut response = Response::from_json(&token)?;
            response.headers_mut().set("Cache-Control", "no-store")?;
            Ok(response)
        })
        .post_async("/v1/oauth/refresh", |mut req, ctx| async move {
            if let Some(response) = authorize_request(&req, &ctx)? {
                return Ok(response);
            }

            let payload = match req.json::<RefreshRequest>().await {
                Ok(payload) => payload,
                Err(_) => return Response::error("Invalid JSON body", 400),
            };

            if is_blank(&payload.refresh_token) {
                return Response::error("Missing refresh_token", 400);
            }

            let body = format!(
                "grant_type=refresh_token&refresh_token={}",
                urlencoding::encode(payload.refresh_token.trim())
            );

            let token = exchange_token(&ctx, body).await?;
            let mut response = Response::from_json(&token)?;
            response.headers_mut().set("Cache-Control", "no-store")?;
            Ok(response)
        })
        .run(req, env)
        .await
}

fn authorize_request(req: &Request, ctx: &RouteContext<()>) -> Result<Option<Response>> {
    let expected_key = match ctx.var("BROKER_API_KEY") {
        Ok(value) => value.to_string(),
        Err(_) => return Ok(None),
    };

    let provided = req
        .headers()
        .get("x-broker-key")?
        .unwrap_or_default()
        .trim()
        .to_string();

    if provided != expected_key {
        return Ok(Some(Response::error("Unauthorized", 401)?));
    }

    Ok(None)
}

async fn exchange_token(ctx: &RouteContext<()>, body: String) -> Result<TickTickTokenResponse> {
    let client_id = ctx.secret("TICKTICK_CLIENT_ID")?.to_string();
    let client_secret = ctx.secret("TICKTICK_CLIENT_SECRET")?.to_string();

    let basic_auth = format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("{}:{}", client_id, client_secret))
    );

    let headers = Headers::new();
    headers.set("Authorization", &basic_auth)?;
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;

    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    init.with_headers(headers);
    init.with_body(Some(body.into()));

    let request = Request::new_with_init(TICKTICK_TOKEN_URL, &init)?;
    let mut upstream = Fetch::Request(request).send().await?;

    let status = upstream.status_code();
    if status >= 400 {
        let details = upstream
            .text()
            .await
            .unwrap_or_else(|_| "Token exchange failed".to_string());
        return Err(Error::RustError(format!(
            "TickTick token endpoint returned {}: {}",
            status, details
        )));
    }

    upstream
        .json::<TickTickTokenResponse>()
        .await
        .map_err(|err| Error::RustError(format!("Failed to parse token response: {err}")))
}

fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}
