mod api;
mod cli;
mod config;
mod models;
mod output;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}
