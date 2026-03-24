# Development

This document covers setup that is not needed for the default end-user install flow.

## Local development

Clone the repo and install from source:

```bash
git clone <repo-url>
cd ticktick-cli
cargo install --path .
```

Run tests:

```bash
cargo test
```

## OAuth configuration

The CLI defaults to the shared brokered auth flow:

- `TICKTICK_CLIENT_ID=Ul8jc7U2kv5DwjN6Uw`
- `TICKTICK_OAUTH_BROKER_URL=https://ticktick-auth-broker.carter-tran.workers.dev`
- `TICKTICK_REDIRECT_URI=http://localhost:8080/callback`

You only need to set environment variables when you want to override that behavior.

### Option A: bring your own TickTick app

Use the official TickTick Open API docs:

- OpenAPI docs: https://developer.ticktick.com/docs/index.html#/openapi
- Developer Center: https://developer.ticktick.com/

Create an app, copy your `client_id` and `client_secret`, and add your redirect URL in the TickTick Developer Center. It must match `TICKTICK_REDIRECT_URI`.

Example:

```bash
export TICKTICK_CLIENT_ID="<your-client-id>"
export TICKTICK_CLIENT_SECRET="<your-client-secret>"
export TICKTICK_REDIRECT_URI="http://localhost:8080/callback"
```

If `TICKTICK_CLIENT_SECRET` is set and `TICKTICK_OAUTH_BROKER_URL` is unset, the CLI uses direct OAuth with your app credentials.

### Option B: override the shared broker

Example:

```bash
unset TICKTICK_CLIENT_SECRET
export TICKTICK_CLIENT_ID="Ul8jc7U2kv5DwjN6Uw"
export TICKTICK_OAUTH_BROKER_URL="https://<your-worker-domain>"
export TICKTICK_REDIRECT_URI="http://localhost:8080/callback"
```

### Environment file reference

See `.env.example` for the full set of supported environment variables.

## Auth broker deployment

The broker implementation lives in `cloudflare-auth-broker/`.

Its setup, Cloudflare secrets, deployment steps, local development workflow, and API contract are documented in `cloudflare-auth-broker/README.md`.
