# Cloudflare TickTick Auth Broker (Rust)

This is a minimal Cloudflare Worker that keeps your TickTick `client_secret` off end-user machines.

It exposes two endpoints:

- `POST /v1/oauth/exchange` to exchange `code + code_verifier` for tokens.
- `POST /v1/oauth/refresh` to refresh an access token.

The worker does not store user tokens. It only proxies token calls to TickTick.

## Required Cloudflare secrets

Set these with `wrangler secret put`:

- `TICKTICK_CLIENT_ID`
- `TICKTICK_CLIENT_SECRET`

Optional hardening:

- `BROKER_API_KEY` (if set, every request must include header `x-broker-key`)

## Deploy

From this directory:

```bash
cd cloudflare-auth-broker
npm install -g wrangler
wrangler secret put TICKTICK_CLIENT_ID
wrangler secret put TICKTICK_CLIENT_SECRET
# optional
wrangler secret put BROKER_API_KEY
wrangler deploy
```

## Local dev

```bash
wrangler dev
```

If you test locally with `.dev.vars`, do not commit it.

## API contract

### `POST /v1/oauth/exchange`

Request JSON:

```json
{
  "code": "authorization-code",
  "code_verifier": "pkce-verifier",
  "redirect_uri": "http://localhost:8080/callback"
}
```

### `POST /v1/oauth/refresh`

Request JSON:

```json
{
  "refresh_token": "refresh-token"
}
```

Response JSON for both endpoints is the TickTick token payload (forwarded).

## Example curl

```bash
curl -X POST "https://<your-worker>/v1/oauth/exchange" \
  -H "content-type: application/json" \
  -H "x-broker-key: <optional-key>" \
  -d '{
    "code": "<code>",
    "code_verifier": "<code-verifier>",
    "redirect_uri": "http://localhost:8080/callback"
  }'
```

## Notes

- Keep this broker endpoint behind Cloudflare rate limits.
- Keep logging minimal to avoid leaking token data.
- If you distribute a public CLI, do not embed `BROKER_API_KEY` as a real secret.

## CLI integration

For `ticktick-cli`, users can set:

```bash
export TICKTICK_CLIENT_ID="<shared-client-id>"
export TICKTICK_OAUTH_BROKER_URL="https://<your-worker-domain>"
# optional
export TICKTICK_OAUTH_BROKER_KEY="<broker-key>"
```

`TICKTICK_CLIENT_SECRET` is not required on user machines when broker mode is used.
