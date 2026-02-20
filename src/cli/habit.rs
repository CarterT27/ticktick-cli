use crate::api::TickTickClient;
use crate::config::AppConfig;
use crate::output::{print_habits, OutputFormat};
use anyhow::Result;
use clap::{Args, Subcommand};
use uuid::Uuid;

#[derive(Subcommand)]
pub enum HabitCommands {
    Add(HabitAddArgs),
    List(HabitListArgs),
    Update(HabitUpdateArgs),
    Delete(HabitDeleteArgs),
}

#[derive(Args)]
pub struct HabitAddArgs {
    title: String,
    #[arg(long)]
    content: Option<String>,
    #[arg(long)]
    goal: Option<i32>,
    #[arg(long)]
    unit: Option<String>,
    #[arg(long)]
    days: Option<String>,
    #[arg(long)]
    remind: Option<i32>,
    #[arg(long)]
    repeat: Option<String>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn habit_add(args: HabitAddArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let days = args
        .days
        .map(|d| d.split(',').filter_map(|s| s.trim().parse().ok()).collect());

    let habit = crate::models::Habit {
        id: Uuid::new_v4().to_string(),
        title: args.title,
        content: args.content,
        goal: args.goal,
        unit: args.unit,
        days,
        remind: args.remind,
        repeat: args.repeat,
        repeated: Some(true),
        ..Default::default()
    };

    client.create_habit(&habit).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&habit)?);
        }
        OutputFormat::Human => {
            println!("Habit created: {}", habit.title);
        }
    }

    Ok(())
}

#[derive(Args)]
pub struct HabitListArgs {
    #[arg(long)]
    name: Option<String>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn habit_list(args: HabitListArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut habits = client.get_habits().await?;

    if let Some(name) = args.name {
        habits.retain(|h| h.title.contains(&name));
    }

    print_habits(&habits, args.output);
    Ok(())
}

#[derive(Args)]
pub struct HabitUpdateArgs {
    habit_id: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    content: Option<String>,
    #[arg(long)]
    goal: Option<i32>,
    #[arg(long)]
    unit: Option<String>,
    #[arg(long)]
    days: Option<String>,
    #[arg(long)]
    remind: Option<i32>,
    #[arg(long)]
    repeat: Option<String>,
}

pub async fn habit_update(args: HabitUpdateArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let habits = client.get_habits().await?;
    let mut habit = habits
        .into_iter()
        .find(|h| h.id == args.habit_id)
        .ok_or_else(|| anyhow::anyhow!("Habit not found: {}", args.habit_id))?;

    if let Some(title) = args.title {
        habit.title = title;
    }
    if let Some(content) = args.content {
        habit.content = Some(content);
    }
    if let Some(goal) = args.goal {
        habit.goal = Some(goal);
    }
    if let Some(unit) = args.unit {
        habit.unit = Some(unit);
    }
    if let Some(days) = args.days {
        habit.days = Some(
            days.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect(),
        );
    }
    if let Some(remind) = args.remind {
        habit.remind = Some(remind);
    }
    if let Some(repeat) = args.repeat {
        habit.repeat = Some(repeat);
    }

    client.update_habit(&habit).await?;
    println!("Habit updated: {}", habit.title);
    Ok(())
}

#[derive(Args)]
pub struct HabitDeleteArgs {
    habit_id: String,
    #[arg(long, default_value = "true")]
    confirm: bool,
}

pub async fn habit_delete(args: HabitDeleteArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let habits = client.get_habits().await?;
    let habit = habits
        .into_iter()
        .find(|h| h.id == args.habit_id)
        .ok_or_else(|| anyhow::anyhow!("Habit not found: {}", args.habit_id))?;

    if args.confirm {
        println!(
            "Are you sure you want to delete habit '{}'? [y/N]",
            habit.title
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    client.delete_habit(&args.habit_id).await?;
    println!("Habit deleted: {}", habit.title);
    Ok(())
}
