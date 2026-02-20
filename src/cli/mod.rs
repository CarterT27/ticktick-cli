mod auth;
mod project;
mod task;

pub use auth::*;
pub use project::*;
pub use task::*;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tt")]
#[command(about = "A fast, snappy TickTick CLI tool", long_about = None)]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Auth {
        #[command(subcommand)]
        subcommand: auth::AuthCommands,
    },
    Task {
        #[command(subcommand)]
        subcommand: task::TaskCommands,
    },
    Project {
        #[command(subcommand)]
        subcommand: project::ProjectCommands,
    },
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Auth { subcommand } => match subcommand {
            auth::AuthCommands::Login => login().await,
            auth::AuthCommands::Logout => logout().await,
            auth::AuthCommands::Status => status().await,
        },
        Commands::Task { subcommand } => match subcommand {
            task::TaskCommands::Add(args) => task_add(args).await,
            task::TaskCommands::List(args) => task_list(args).await,
            task::TaskCommands::Update(args) => task_update(args).await,
            task::TaskCommands::Complete(args) => task_complete(args).await,
            task::TaskCommands::Delete(args) => task_delete(args).await,
        },
        Commands::Project { subcommand } => match subcommand {
            project::ProjectCommands::Add(args) => project_add(args).await,
            project::ProjectCommands::List(args) => project_list(args).await,
            project::ProjectCommands::Get(args) => project_get(args).await,
            project::ProjectCommands::Data(args) => project_data(args).await,
            project::ProjectCommands::Update(args) => project_update(args).await,
            project::ProjectCommands::Delete(args) => project_delete(args).await,
        },
    }
}
