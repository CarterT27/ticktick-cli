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
  - Shared auth broker (defaults provided out of the box)
- Optional redirect override:
  - `TICKTICK_REDIRECT_URI` (defaults to `http://localhost:8080/callback`)

## Setup TickTick OAuth (first-time)

Use the official TickTick Open API docs:

- OpenAPI docs: https://developer.ticktick.com/docs/index.html#/openapi
- Developer Center (then click **Manage Apps**): https://developer.ticktick.com/

From there, create an app and copy your `client_id` and `client_secret`.
Also add your redirect URL in the TickTick Developer Center app settings (for example `http://localhost:8080/callback`), and make sure it matches `TICKTICK_REDIRECT_URI` if you set that variable.

If you use the shared auth broker in this repo, the CLI now defaults to:

- `TICKTICK_CLIENT_ID=Ul8jc7U2kv5DwjN6Uw`
- `TICKTICK_OAUTH_BROKER_URL=https://ticktick-auth-broker.carter-tran.workers.dev`

Users only need to override those variables if they want to use a different app or a different broker.

## Optional: Cloudflare auth broker

For distribution where users should not manage `TICKTICK_CLIENT_SECRET`, deploy the lightweight broker in `cloudflare-auth-broker/`.

See `cloudflare-auth-broker/README.md` for setup and deploy steps.

## Install

### Install with Homebrew

Install the latest tagged release from the tap:

```bash
brew tap CarterT27/tap
brew install CarterT27/tap/ticktick-cli
```

### Build it Yourself

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

If `TICKTICK_CLIENT_SECRET` is set and `TICKTICK_OAUTH_BROKER_URL` is not, the CLI uses direct OAuth with your app credentials.

Shared broker mode (no local client secret required):

```bash
unset TICKTICK_CLIENT_SECRET
export TICKTICK_CLIENT_ID="Ul8jc7U2kv5DwjN6Uw"
export TICKTICK_OAUTH_BROKER_URL="https://ticktick-auth-broker.carter-tran.workers.dev"
```

Those two variables are now the CLI defaults, so for the shared broker you can also just run:

```bash
unset TICKTICK_CLIENT_SECRET
```

Optional overrides:

```bash
export TICKTICK_REDIRECT_URI="http://localhost:8080/callback"
export TICKTICK_OAUTH_BROKER_URL="https://<your-worker-domain>"
```

3. Install `tt` from this repo:

```bash
cargo install --path .
```

4. Authenticate:

```bash
tt auth login
```

Top-level aliases also work:

```bash
tt login
tt status
tt logout
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
