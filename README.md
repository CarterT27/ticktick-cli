# ticktick-cli

`ticktick-cli` is a Rust command-line client for working with TickTick from your terminal.

This project is actively evolving. Expect commands and behavior to improve over time, but the core goal is stable: fast task and project workflows without leaving the shell.

## What it does (today)

- Authenticate with TickTick via OAuth.
- List, create, update, complete, and delete tasks.
- List, create, inspect, update, and delete projects.
- Provide both human-friendly and JSON output modes for many commands.

## Prerequisites

- Rust toolchain (`rustc` + `cargo`) installed.
- OAuth configuration via one of these modes:
  - Bring-your-own TickTick OAuth app (`TICKTICK_CLIENT_ID` + `TICKTICK_CLIENT_SECRET`)
  - Shared auth broker (`TICKTICK_CLIENT_ID` + `TICKTICK_OAUTH_BROKER_URL`)
- Optional redirect override:
  - `TICKTICK_REDIRECT_URI` (defaults to `http://localhost:8080/callback`)

## Setup TickTick OAuth (first-time)

Use the official TickTick Open API docs:

- OpenAPI docs: https://developer.ticktick.com/docs/index.html#/openapi
- Developer Center (then click **Manage Apps**): https://developer.ticktick.com/

From there, create an app and copy your `client_id` and `client_secret`.

If you use a shared auth broker, users only need your shared `client_id` and broker URL.

## Optional: Cloudflare auth broker

For distribution where users should not manage `TICKTICK_CLIENT_SECRET`, deploy the lightweight broker in `cloudflare-auth-broker/`.

See `cloudflare-auth-broker/README.md` for setup and deploy steps.

## Install

1. Clone the repo:

```bash
git clone <repo-url>
cd ticktick-cli
```

2. Export environment variables.

BYO credentials mode:

```bash
export TICKTICK_CLIENT_ID="<your-client-id>"
export TICKTICK_CLIENT_SECRET="<your-client-secret>"
# optional (default is shown)
export TICKTICK_REDIRECT_URI="http://localhost:8080/callback"
```

Shared broker mode (no local client secret required):

```bash
export TICKTICK_CLIENT_ID="<shared-client-id>"
export TICKTICK_OAUTH_BROKER_URL="https://<your-worker-domain>"
# optional broker header key if your broker enforces one
export TICKTICK_OAUTH_BROKER_KEY="<broker-key>"
# optional (default is shown)
export TICKTICK_REDIRECT_URI="http://localhost:8080/callback"
```

3. Install `tt` from this repo:

```bash
cargo install --path .
```

4. Authenticate:

```bash
tt login
```

After login, credentials are stored in the app config directory used by your OS (the CLI prints the exact file path after successful auth).

## Quick start

Show help:

```bash
tt --help
```

Common task flows:

```bash
# add a task
tt add "Write release notes"

# list tasks
tt ls

# filter examples
tt ls --status open --limit 20
tt task list --when today

# complete or remove
tt done <task-id>
tt rm <task-id>
```

Project flows:

```bash
# list projects
tt projects

# create one
tt project add "Work"

# inspect one
tt project get <project-id>
```

## Development

Run tests:

```bash
cargo test
```

## Notes

- Most commands require authentication first.
- If you prefer structured output, use command variants that support `--output json`.
- For the latest flags and subcommands, use `tt --help`, `tt task --help`, and `tt project --help`.
