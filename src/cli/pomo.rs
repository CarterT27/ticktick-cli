use crate::api::TickTickClient;
use crate::config::AppConfig;
use crate::output::{print_pomodoros, OutputFormat};
use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};
use uuid::Uuid;

#[derive(Subcommand)]
pub enum PomoCommands {
    Start(PomoStartArgs),
    Stop(PomoStopArgs),
    History(PomoHistoryArgs),
}

#[derive(Args)]
pub struct PomoStartArgs {
    #[arg(long)]
    task_id: Option<String>,
    #[arg(long, default_value = "25")]
    duration: i64,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn pomo_start(args: PomoStartArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let pomodoro = crate::models::Pomo {
        id: Some(Uuid::new_v4().to_string()),
        started: Some(Utc::now().timestamp_millis()),
        duration: Some(args.duration * 60 * 1000),
        kind: Some("pomo".to_string()),
        task_id: args.task_id,
        ..Default::default()
    };

    let started = client.pomodoros_start(&pomodoro).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&started)?);
        }
        OutputFormat::Human => {
            println!("Pomodoro started for {} minutes", args.duration);
            if let Some(task_id) = &started.task_id {
                println!("Task ID: {}", task_id);
            }
            println!("Pomodoro ID: {}", started.id.unwrap_or_default());
        }
    }

    Ok(())
}

#[derive(Args)]
pub struct PomoStopArgs {
    #[arg(short, long)]
    pomo_id: String,
    #[arg(short, long)]
    task_id: String,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn pomo_stop(args: PomoStopArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let stopped = client.pomodoros_stop(&args.pomo_id, &args.task_id).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&stopped)?);
        }
        OutputFormat::Human => {
            println!("Pomodoro stopped");
            if let Some(duration) = stopped.duration {
                let minutes = duration / (60 * 1000);
                println!("Duration: {} minutes", minutes);
            }
        }
    }

    Ok(())
}

#[derive(Args)]
pub struct PomoHistoryArgs {
    #[arg(short, long)]
    task_id: Option<String>,
    #[arg(long, default_value = "50")]
    limit: usize,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn pomo_history(args: PomoHistoryArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut pomodoros = client.pomodoros_history(args.task_id).await?;
    pomodoros = pomodoros.into_iter().take(args.limit).collect();

    print_pomodoros(&pomodoros, args.output);
    Ok(())
}
