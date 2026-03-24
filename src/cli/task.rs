mod dates;
mod filters;
mod projects;

#[cfg(test)]
mod tests;

use self::dates::{
    extract_due_date_from_input, format_ticktick_due_date, normalize_task_datetime_input,
    task_matches_when_filter, TaskWhenFilter,
};
use self::filters::{
    extract_implicit_list_from_terms, is_inbox_list_name, merge_tags, parse_priority_value,
    parse_shorthand, parse_task_add_shorthand, task_has_all_tags,
};
use self::projects::{
    cache_store, forget_task_project_id, get_tasks_across_projects, get_tasks_for_project,
    infer_default_project_id, remember_task, remember_task_project_id, remember_tasks,
    resolve_project_id, resolve_task_project_id,
};
use crate::api::TickTickClient;
use crate::config::AppConfig;
use crate::models::{Task, TaskStatus};
use crate::output::{print_tasks, OutputFormat};
use anyhow::{anyhow, Result};
use atty::Stream;
use chrono::Local;
use clap::{Args, Subcommand};
use std::io::{self, Read};

#[derive(Subcommand)]
pub enum TaskCommands {
    #[command(alias = "new")]
    Add(TaskAddArgs),
    #[command(alias = "ls")]
    List(TaskListArgs),
    #[command(alias = "edit")]
    Update(TaskUpdateArgs),
    #[command(alias = "done")]
    Complete(TaskCompleteArgs),
    #[command(aliases = ["rm", "del"])]
    Delete(TaskDeleteArgs),
}

fn resolve_task_note_fields(
    content: Option<String>,
    desc: Option<String>,
) -> (Option<String>, Option<String>) {
    match (content, desc) {
        (Some(content), Some(desc)) => (Some(content), Some(desc)),
        (Some(value), None) | (None, Some(value)) => (Some(value.clone()), Some(value)),
        (None, None) => (None, None),
    }
}

fn sync_task_note_fields(task: &mut Task) {
    match (&task.content, &task.desc) {
        (Some(content), None) => {
            task.desc = Some(content.clone());
        }
        (None, Some(desc)) => {
            task.content = Some(desc.clone());
        }
        _ => {}
    }
}

fn task_is_completed(task: &Task) -> bool {
    matches!(task.status, Some(TaskStatus::Completed))
}

#[derive(Args)]
pub struct TaskAddArgs {
    title: Vec<String>,
    #[arg(long, help = "Visible task note shown in TickTick")]
    content: Option<String>,
    #[arg(
        long,
        help = "Secondary TickTick API description field; mirrored to content when used alone"
    )]
    desc: Option<String>,
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    list: Option<String>,
    #[arg(long, value_parser = normalize_task_datetime_input)]
    start_date: Option<String>,
    #[arg(long, value_parser = normalize_task_datetime_input)]
    due_date: Option<String>,
    #[arg(long)]
    time_zone: Option<String>,
    #[arg(long)]
    all_day: Option<bool>,
    #[arg(long, value_parser = parse_priority_value)]
    priority: Option<i32>,
    #[arg(long)]
    tags: Vec<String>,
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
    let mut args = args;
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;
    let cache = cache_store();

    let raw_input = if args.stdin || (!atty::is(Stream::Stdin) && args.title.is_empty()) {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else {
        args.title.join(" ")
    };

    let today = Local::now().date_naive();
    let (input_without_due_date, inferred_due_date) =
        extract_due_date_from_input(&raw_input, today);
    let shorthand = parse_task_add_shorthand(&input_without_due_date);

    if args.priority.is_none() {
        args.priority = shorthand.priority;
    }
    if args.list.is_none() {
        args.list = shorthand.list;
    }
    if args.due_date.is_none() {
        if let Some(date) = inferred_due_date {
            let formatted = format_ticktick_due_date(date)
                .ok_or_else(|| anyhow!("Failed to format inferred due date '{}'", date))?;
            args.due_date = Some(formatted.clone());
            if args.start_date.is_none() {
                args.start_date = Some(formatted);
            }
            if args.all_day.is_none() {
                args.all_day = Some(true);
            }
        }
    }
    merge_tags(&mut args.tags, shorthand.tags);

    let title = shorthand.terms.join(" ").trim().to_string();
    if title.is_empty() {
        return Err(anyhow!("Task title required or provide stdin"));
    }

    let project_id =
        match resolve_project_id(&client, cache.as_ref(), args.project_id, args.list).await? {
            Some(project_id) => project_id,
            None => infer_default_project_id(&client, cache.as_ref()).await?,
        };

    let (content, desc) = resolve_task_note_fields(args.content, args.desc);

    let task = crate::models::Task {
        id: None,
        title,
        content,
        desc,
        project_id: Some(project_id.clone()),
        start_date: args.start_date,
        due_date: args.due_date,
        time_zone: args.time_zone,
        is_all_day: args.all_day,
        priority: args.priority.or(Some(0)),
        tags: if args.tags.is_empty() {
            None
        } else {
            Some(args.tags)
        },
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
    let mut task = task;
    sync_task_note_fields(&mut task);

    let created = client.create_task(&task).await?;
    remember_task(cache.as_ref(), &created, Some(&project_id));

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
    #[arg(long, value_parser = parse_priority_value)]
    priority: Option<i32>,
    #[arg(long)]
    tags: Vec<String>,
    #[arg(long, value_enum)]
    when: Option<TaskWhenFilter>,
    #[arg(long, default_value = "0")]
    limit: usize,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
    query: Vec<String>,
}

pub async fn task_list(args: TaskListArgs) -> Result<()> {
    let mut args = args;
    let app_config = AppConfig::new()?;
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;
    let cache = cache_store();

    let shorthand = parse_shorthand(&args.query.join(" "));
    if args.priority.is_none() {
        args.priority = shorthand.priority;
    }
    if args.project_id.is_none() && args.list.is_none() {
        args.list = shorthand.list;
    }
    if args.when.is_none() {
        args.when = shorthand.when;
    }
    merge_tags(&mut args.tags, shorthand.tags);
    let mut search_terms = shorthand.terms;

    if args.project_id.is_none() && args.list.is_none() {
        if let Some(list_name) = extract_implicit_list_from_terms(&mut search_terms) {
            args.list = Some(list_name);
        }
    }

    if args.project_id.is_none()
        && args.list.is_none()
        && search_terms.len() == 1
        && search_terms
            .first()
            .is_some_and(|term| is_inbox_list_name(term))
    {
        args.list = search_terms.pop();
    }

    let inbox_only =
        args.project_id.is_none() && args.list.as_deref().is_some_and(is_inbox_list_name);

    let project_id = if inbox_only {
        None
    } else {
        resolve_project_id(&client, cache.as_ref(), args.project_id, args.list.clone()).await?
    };

    let mut tasks = if inbox_only {
        get_tasks_for_project(&client, "").await?
    } else if let Some(ref project_id) = project_id {
        get_tasks_for_project(&client, project_id).await?
    } else {
        get_tasks_across_projects(&client, cache.as_ref()).await?
    };
    remember_tasks(cache.as_ref(), &tasks, project_id.as_deref());

    if let Some(status) = args.status {
        let normalized = status.to_ascii_lowercase();
        let is_done = match normalized.as_str() {
            "done" | "completed" | "complete" => true,
            "todo" | "open" | "normal" | "active" => false,
            _ => {
                return Err(anyhow!(
                    "Unsupported status '{}'. Use one of: done, completed, todo, open",
                    status
                ));
            }
        };

        tasks.retain(|task| {
            if is_done {
                task_is_completed(task)
            } else {
                !task_is_completed(task)
            }
        });
    }

    if let Some(prio) = args.priority {
        tasks.retain(|task| task.priority.unwrap_or(0) == prio);
    }

    if !args.tags.is_empty() {
        tasks.retain(|task| task_has_all_tags(task, &args.tags));
    }

    if let Some(when) = args.when {
        let today = Local::now().date_naive();
        tasks.retain(|task| task_matches_when_filter(task, when, today));
    }

    if !search_terms.is_empty() {
        let needles: Vec<String> = search_terms
            .into_iter()
            .map(|term| term.to_ascii_lowercase())
            .collect();
        tasks.retain(|task| {
            let haystack = format!(
                "{} {} {}",
                task.title,
                task.content.as_deref().unwrap_or_default(),
                task.desc.as_deref().unwrap_or_default()
            )
            .to_ascii_lowercase();
            needles.iter().all(|needle| haystack.contains(needle))
        });
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
    #[arg(long, help = "Visible task note shown in TickTick")]
    content: Option<String>,
    #[arg(
        long,
        help = "Secondary TickTick API description field; mirrored to content when used alone"
    )]
    desc: Option<String>,
    #[arg(long, value_parser = normalize_task_datetime_input)]
    start_date: Option<String>,
    #[arg(long, value_parser = normalize_task_datetime_input)]
    due_date: Option<String>,
    #[arg(long)]
    time_zone: Option<String>,
    #[arg(long, value_parser = parse_priority_value)]
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
    let cache = cache_store();
    let explicit_scope = args.project_id.is_some() || args.list.is_some();

    let mut resolved = resolve_task_project_id(
        &client,
        cache.as_ref(),
        &args.task_id,
        args.project_id.clone(),
        args.list.clone(),
    )
    .await?;

    let mut task = match client.get_task(&resolved.project_id, &args.task_id).await {
        Ok(task) => task,
        Err(_) if resolved.from_cache && !explicit_scope => {
            forget_task_project_id(cache.as_ref(), &args.task_id);
            resolved =
                resolve_task_project_id(&client, cache.as_ref(), &args.task_id, None, None).await?;
            client.get_task(&resolved.project_id, &args.task_id).await?
        }
        Err(err) => return Err(err),
    };

    if let Some(title) = args.title {
        task.title = title;
    }
    let note_fields_were_updated = args.content.is_some() || args.desc.is_some();
    let (content, desc) = resolve_task_note_fields(args.content, args.desc);
    if let Some(content) = content {
        task.content = Some(content);
    }
    if let Some(desc) = desc {
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
    if note_fields_were_updated {
        if task.content.is_none() {
            task.content = task.desc.clone();
        }
        if task.desc.is_none() {
            task.desc = task.content.clone();
        }
    } else {
        sync_task_note_fields(&mut task);
    }

    let updated = client.update_task(&args.task_id, &task).await?;
    remember_task(cache.as_ref(), &updated, Some(&resolved.project_id));

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
    let cache = cache_store();
    let explicit_scope = args.project_id.is_some() || args.list.is_some();

    let mut resolved = resolve_task_project_id(
        &client,
        cache.as_ref(),
        &args.task_id,
        args.project_id,
        args.list,
    )
    .await?;

    if let Err(err) = client
        .complete_task(&resolved.project_id, &args.task_id)
        .await
    {
        if resolved.from_cache && !explicit_scope {
            forget_task_project_id(cache.as_ref(), &args.task_id);
            resolved =
                resolve_task_project_id(&client, cache.as_ref(), &args.task_id, None, None).await?;
            client
                .complete_task(&resolved.project_id, &args.task_id)
                .await?;
        } else {
            return Err(err);
        }
    }
    remember_task_project_id(cache.as_ref(), &args.task_id, &resolved.project_id);

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
    let cache = cache_store();
    let explicit_scope = args.project_id.is_some() || args.list.is_some();
    let mut resolved = resolve_task_project_id(
        &client,
        cache.as_ref(),
        &args.task_id,
        args.project_id,
        args.list,
    )
    .await?;

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

    if let Err(err) = client
        .delete_task(&resolved.project_id, &args.task_id)
        .await
    {
        if resolved.from_cache && !explicit_scope {
            forget_task_project_id(cache.as_ref(), &args.task_id);
            resolved =
                resolve_task_project_id(&client, cache.as_ref(), &args.task_id, None, None).await?;
            client
                .delete_task(&resolved.project_id, &args.task_id)
                .await?;
        } else {
            return Err(err);
        }
    }
    forget_task_project_id(cache.as_ref(), &args.task_id);
    println!("Task deleted: {}", args.task_id);

    Ok(())
}
