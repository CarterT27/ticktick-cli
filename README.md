# ticktick-cli

`ticktick-cli` is a Rust command-line client for working with TickTick from your terminal.

## What it does

- Authenticate with TickTick via OAuth
- List, create, update, complete, and delete tasks
- List, create, inspect, update, and delete projects
- Provide human-friendly and JSON output modes for many commands

## Install

```bash
brew tap CarterT27/tap
brew install CarterT27/tap/ticktick-cli
```

## First-time login

The default install path uses the shared brokered OAuth flow. You do not need to create your own TickTick app or set a client secret.

Run:

```bash
tt auth login
```

Top-level aliases also work:

```bash
tt login
tt status
tt logout
```

The browser-based login flow uses these defaults automatically:

- `TICKTICK_CLIENT_ID=Ul8jc7U2kv5DwjN6Uw`
- `TICKTICK_OAUTH_BROKER_URL=https://ticktick-auth-broker.carter-tran.workers.dev`
- `TICKTICK_REDIRECT_URI=http://localhost:8080/callback`

You only need to set environment variables if you want to override those defaults.

After login, credentials are stored in the app config directory for your OS. The CLI prints the exact path after successful auth.

## Quick start

Authenticate:

```bash
tt auth login
```

Then use it:

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

# add a section within a project
tt project section add <project-id> "Backlog"
```

## Development

Developer setup, alternate OAuth modes, and broker deployment notes are in `docs/development.md`.

Run tests with:

```bash
cargo test
```
