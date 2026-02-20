use crate::api::TickTickClient;
use crate::config::AppConfig;
use crate::output::{print_tasks, OutputFormat};
use anyhow::Result;
use atty::Stream;
use clap::{Args, Subcommand};
use std::io::{self, Read};

#[derive(Subcommand)]
pub enum TaskCommands {
    Add(TaskAddArgs),
    List(TaskListArgs),
    Update(TaskUpdateArgs),
    Complete(TaskCompleteArgs),
    Delete(TaskDeleteArgs),
}

#[derive(Args)]
pub struct TaskAddArgs {
    title: Option<String>,
    #[arg(long)]
    content: Option<String>,
    #[arg(long)]
    desc: Option<String>,
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    list: Option<String>,
    #[arg(long)]
    start_date: Option<String>,
    #[arg(long)]
    due_date: Option<String>,
    #[arg(long)]
    time_zone: Option<String>,
    #[arg(long)]
    all_day: Option<bool>,
    #[arg(long)]
    priority: Option<i32>,
    #[arg(long)]
    reminders: Vec<String>,
    #[arg(long)]
    repeat_flag: Option<String>,
    #[arg(long)]
    sort_order: Option<i64>,
    #[arg(long)]
    stdin: bool,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn task_add(args: TaskAddArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let title = if args.stdin || (!atty::is(Stream::Stdin) && args.title.is_none()) {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer.trim().to_string()
    } else {
        args.title
            .ok_or_else(|| anyhow::anyhow!("Task title required or provide stdin"))?
    };

    let mut project_id = args.project_id;
    if project_id.is_none() {
        if let Some(list_name) = args.list {
            let projects = client.get_projects().await?;
            let project = projects
                .iter()
                .find(|p| p.name == list_name)
                .ok_or_else(|| anyhow::anyhow!("List not found: {}", list_name))?;
            project_id = project.id.clone();
        }
    }

    let project_id =
        project_id.ok_or_else(|| anyhow::anyhow!("Project ID or list name required"))?;

    let task = crate::models::Task {
        id: None,
        title,
        content: args.content,
        desc: args.desc,
        project_id: Some(project_id),
        start_date: args.start_date,
        due_date: args.due_date,
        time_zone: args.time_zone,
        is_all_day: args.all_day,
        priority: args.priority.or(Some(0)),
        reminders: if args.reminders.is_empty() {
            None
        } else {
            Some(args.reminders)
        },
        repeat_flag: args.repeat_flag,
        sort_order: args.sort_order,
        kind: Some("TASK".to_string()),
        ..Default::default()
    };

    let created = client.create_task(&task).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&created)?);
        }
        OutputFormat::Human => {
            println!("Task created: {}", created.title);
            println!("ID: {}", created.id.clone().unwrap_or_default());
        }
    }

    Ok(())
}

#[derive(Args)]
pub struct TaskListArgs {
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    list: Option<String>,
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    priority: Option<i32>,
    #[arg(long, default_value = "0")]
    limit: usize,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn task_list(args: TaskListArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut project_id = args.project_id;
    if project_id.is_none() {
        if let Some(list_name) = args.list {
            let projects = client.get_projects().await?;
            let project = projects
                .iter()
                .find(|p| p.name == list_name)
                .ok_or_else(|| anyhow::anyhow!("List not found: {}", list_name))?;
            project_id = project.id.clone();
        }
    }

    let project_id =
        project_id.ok_or_else(|| anyhow::anyhow!("Project ID or list name required"))?;

    let data = client.get_project_data(&project_id).await?;
    let mut tasks = data.tasks.unwrap_or_default();

    if let Some(status) = args.status {
        let is_done = status == "done" || status == "completed";
        tasks.retain(|t| {
            t.status.as_ref().map_or(false, |s| match s {
                crate::models::TaskStatus::Completed => is_done,
                crate::models::TaskStatus::Normal => !is_done,
            })
        });
    }

    if let Some(prio) = args.priority {
        tasks.retain(|t| t.priority.unwrap_or(0) == prio);
    }

    if args.limit > 0 {
        tasks = tasks.into_iter().take(args.limit).collect();
    }

    print_tasks(&tasks, args.output);
    Ok(())
}

#[derive(Args)]
pub struct TaskUpdateArgs {
    task_id: String,
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    list: Option<String>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    content: Option<String>,
    #[arg(long)]
    desc: Option<String>,
    #[arg(long)]
    start_date: Option<String>,
    #[arg(long)]
    due_date: Option<String>,
    #[arg(long)]
    time_zone: Option<String>,
    #[arg(long)]
    priority: Option<i32>,
    #[arg(long)]
    reminders: Vec<String>,
    #[arg(long)]
    repeat_flag: Option<String>,
    #[arg(long)]
    sort_order: Option<i64>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn task_update(args: TaskUpdateArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut project_id = args.project_id;
    if project_id.is_none() {
        if let Some(list_name) = args.list {
            let projects = client.get_projects().await?;
            let project = projects
                .iter()
                .find(|p| p.name == list_name)
                .ok_or_else(|| anyhow::anyhow!("List not found: {}", list_name))?;
            project_id = project.id.clone();
        }
    }

    let project_id =
        project_id.ok_or_else(|| anyhow::anyhow!("Project ID or list name required"))?;

    let mut task = client.get_task(&project_id, &args.task_id).await?;

    if let Some(title) = args.title {
        task.title = title;
    }
    if let Some(content) = args.content {
        task.content = Some(content);
    }
    if let Some(desc) = args.desc {
        task.desc = Some(desc);
    }
    if let Some(start_date) = args.start_date {
        task.start_date = Some(start_date);
    }
    if let Some(due_date) = args.due_date {
        task.due_date = Some(due_date);
    }
    if let Some(time_zone) = args.time_zone {
        task.time_zone = Some(time_zone);
    }
    if let Some(priority) = args.priority {
        task.priority = Some(priority);
    }
    if !args.reminders.is_empty() {
        task.reminders = Some(args.reminders);
    }
    if let Some(repeat_flag) = args.repeat_flag {
        task.repeat_flag = Some(repeat_flag);
    }
    if let Some(sort_order) = args.sort_order {
        task.sort_order = Some(sort_order);
    }

    let updated = client.update_task(&args.task_id, &task).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&updated)?);
        }
        OutputFormat::Human => {
            println!("Task updated: {}", updated.title);
        }
    }

    Ok(())
}

#[derive(Args)]
pub struct TaskCompleteArgs {
    task_id: String,
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    list: Option<String>,
    #[arg(long, default_value = "true")]
    output: bool,
}

pub async fn task_complete(args: TaskCompleteArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut project_id = args.project_id;
    if project_id.is_none() {
        if let Some(list_name) = args.list {
            let projects = client.get_projects().await?;
            let project = projects
                .iter()
                .find(|p| p.name == list_name)
                .ok_or_else(|| anyhow::anyhow!("List not found: {}", list_name))?;
            project_id = project.id.clone();
        }
    }

    let project_id =
        project_id.ok_or_else(|| anyhow::anyhow!("Project ID or list name required"))?;

    client.complete_task(&project_id, &args.task_id).await?;

    if args.output {
        println!("Task completed: {}", args.task_id);
    }

    Ok(())
}

#[derive(Args)]
pub struct TaskDeleteArgs {
    task_id: String,
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    list: Option<String>,
    #[arg(long, default_value = "true")]
    confirm: bool,
}

pub async fn task_delete(args: TaskDeleteArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut project_id = args.project_id;
    if project_id.is_none() {
        if let Some(list_name) = args.list {
            let projects = client.get_projects().await?;
            let project = projects
                .iter()
                .find(|p| p.name == list_name)
                .ok_or_else(|| anyhow::anyhow!("List not found: {}", list_name))?;
            project_id = project.id.clone();
        }
    }

    let project_id =
        project_id.ok_or_else(|| anyhow::anyhow!("Project ID or list name required"))?;

    if args.confirm {
        println!(
            "Are you sure you want to delete task '{}'? [y/N]",
            args.task_id
        );
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    client.delete_task(&project_id, &args.task_id).await?;
    println!("Task deleted: {}", args.task_id);

    Ok(())
}
