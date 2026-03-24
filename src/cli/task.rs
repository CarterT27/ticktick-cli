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
    parse_shorthand, parse_task_add_shorthand, parse_task_status_value, task_has_all_tags,
};
use self::projects::{
    cache_store, forget_task_project_id, get_tasks_across_projects, get_tasks_for_project,
    infer_default_project_id, remember_task, remember_task_project_id, remember_tasks,
    resolve_project_id, resolve_task_project_id,
};
use super::bootstrap::authenticated_client;
use crate::models::{Task, TaskStatus};
use crate::output::{print_tasks, OutputFormat};
use anyhow::{anyhow, Result};
use atty::Stream;
use chrono::Local;
use clap::{Args, Subcommand};
use serde_json::Value;
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

#[derive(Debug, Default, Clone, Copy)]
struct TaskUpdateClearFlags {
    start_date: bool,
    due_date: bool,
    time_zone: bool,
    tags: bool,
    reminders: bool,
    repeat_flag: bool,
    sort_order: bool,
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
    let client = authenticated_client()?;
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

    let task = Task {
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

    print!("{}", format_task_create_output(&created, args.output)?);

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
    let client = authenticated_client()?;
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

#[derive(Debug, Args)]
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
    #[arg(
        long,
        value_parser = normalize_task_datetime_input,
        conflicts_with = "clear_start_date"
    )]
    start_date: Option<String>,
    #[arg(
        long,
        value_parser = normalize_task_datetime_input,
        conflicts_with = "clear_due_date"
    )]
    due_date: Option<String>,
    #[arg(long, conflicts_with = "clear_time_zone")]
    time_zone: Option<String>,
    #[arg(long)]
    all_day: Option<bool>,
    #[arg(long, value_parser = parse_priority_value)]
    priority: Option<i32>,
    #[arg(long, conflicts_with = "clear_tags")]
    tags: Vec<String>,
    #[arg(long, conflicts_with = "clear_reminders")]
    reminders: Vec<String>,
    #[arg(long, value_parser = parse_task_status_value)]
    status: Option<TaskStatus>,
    #[arg(long, conflicts_with = "clear_repeat_flag")]
    repeat_flag: Option<String>,
    #[arg(long, conflicts_with = "clear_sort_order")]
    sort_order: Option<i64>,
    #[arg(long)]
    clear_start_date: bool,
    #[arg(long)]
    clear_due_date: bool,
    #[arg(long)]
    clear_time_zone: bool,
    #[arg(long)]
    clear_tags: bool,
    #[arg(long)]
    clear_reminders: bool,
    #[arg(long)]
    clear_repeat_flag: bool,
    #[arg(long)]
    clear_sort_order: bool,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

fn build_task_update_payload(task: &Task, clear_flags: TaskUpdateClearFlags) -> Result<Value> {
    let mut payload = serde_json::to_value(task)?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| anyhow!("Failed to encode task update payload as JSON object"))?;

    if clear_flags.start_date {
        object.insert("startDate".to_string(), Value::Null);
    }
    if clear_flags.due_date {
        object.insert("dueDate".to_string(), Value::Null);
    }
    if clear_flags.time_zone {
        object.insert("timeZone".to_string(), Value::Null);
    }
    if clear_flags.tags {
        object.insert("tags".to_string(), Value::Array(Vec::new()));
    }
    if clear_flags.reminders {
        object.insert("reminders".to_string(), Value::Array(Vec::new()));
    }
    if clear_flags.repeat_flag {
        object.insert("repeatFlag".to_string(), Value::Null);
    }
    if clear_flags.sort_order {
        object.insert("sortOrder".to_string(), Value::Null);
    }

    Ok(payload)
}

pub async fn task_update(args: TaskUpdateArgs) -> Result<()> {
    let TaskUpdateArgs {
        task_id,
        project_id,
        list,
        title,
        content,
        desc,
        start_date,
        due_date,
        time_zone,
        all_day,
        priority,
        tags,
        reminders,
        status,
        repeat_flag,
        sort_order,
        clear_start_date,
        clear_due_date,
        clear_time_zone,
        clear_tags,
        clear_reminders,
        clear_repeat_flag,
        clear_sort_order,
        output,
    } = args;

    let client = authenticated_client()?;
    let cache = cache_store();
    let explicit_scope = project_id.is_some() || list.is_some();

    let mut resolved = resolve_task_project_id(
        &client,
        cache.as_ref(),
        &task_id,
        project_id.clone(),
        list.clone(),
    )
    .await?;

    let mut task = match client.get_task(&resolved.project_id, &task_id).await {
        Ok(task) => task,
        Err(_) if resolved.from_cache && !explicit_scope => {
            forget_task_project_id(cache.as_ref(), &task_id);
            resolved =
                resolve_task_project_id(&client, cache.as_ref(), &task_id, None, None).await?;
            client.get_task(&resolved.project_id, &task_id).await?
        }
        Err(err) => return Err(err),
    };

    if let Some(title) = title {
        task.title = title;
    }
    let note_fields_were_updated = content.is_some() || desc.is_some();
    let (content, desc) = resolve_task_note_fields(content, desc);
    if let Some(content) = content {
        task.content = Some(content);
    }
    if let Some(desc) = desc {
        task.desc = Some(desc);
    }
    if clear_start_date {
        task.start_date = None;
    }
    if let Some(start_date) = start_date {
        task.start_date = Some(start_date);
    }
    if clear_due_date {
        task.due_date = None;
    }
    if let Some(due_date) = due_date {
        task.due_date = Some(due_date);
    }
    if clear_time_zone {
        task.time_zone = None;
    }
    if let Some(time_zone) = time_zone {
        task.time_zone = Some(time_zone);
    }
    if let Some(all_day) = all_day {
        task.is_all_day = Some(all_day);
    }
    if let Some(priority) = priority {
        task.priority = Some(priority);
    }
    if clear_tags {
        task.tags = None;
    }
    if !tags.is_empty() {
        task.tags = Some(tags);
    }
    if clear_reminders {
        task.reminders = None;
    }
    if !reminders.is_empty() {
        task.reminders = Some(reminders);
    }
    if let Some(status) = status {
        task.status = Some(status);
    }
    if clear_repeat_flag {
        task.repeat_flag = None;
    }
    if let Some(repeat_flag) = repeat_flag {
        task.repeat_flag = Some(repeat_flag);
    }
    if clear_sort_order {
        task.sort_order = None;
    }
    if let Some(sort_order) = sort_order {
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

    let payload = build_task_update_payload(
        &task,
        TaskUpdateClearFlags {
            start_date: clear_start_date,
            due_date: clear_due_date,
            time_zone: clear_time_zone,
            tags: clear_tags,
            reminders: clear_reminders,
            repeat_flag: clear_repeat_flag,
            sort_order: clear_sort_order,
        },
    )?;
    let updated = client.update_task(&task_id, &payload).await?;
    remember_task(cache.as_ref(), &updated, Some(&resolved.project_id));

    print!("{}", format_task_update_output(&updated, output)?);

    Ok(())
}

#[derive(Args)]
pub struct TaskCompleteArgs {
    task_id: String,
    #[arg(long)]
    project_id: Option<String>,
    #[arg(long)]
    list: Option<String>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn task_complete(args: TaskCompleteArgs) -> Result<()> {
    let TaskCompleteArgs {
        task_id,
        project_id,
        list,
        output,
    } = args;
    let client = authenticated_client()?;
    let cache = cache_store();
    let explicit_scope = project_id.is_some() || list.is_some();

    let mut resolved =
        resolve_task_project_id(&client, cache.as_ref(), &task_id, project_id, list).await?;

    if let Err(err) = client.complete_task(&resolved.project_id, &task_id).await {
        if resolved.from_cache && !explicit_scope {
            forget_task_project_id(cache.as_ref(), &task_id);
            resolved =
                resolve_task_project_id(&client, cache.as_ref(), &task_id, None, None).await?;
            client.complete_task(&resolved.project_id, &task_id).await?;
        } else {
            return Err(err);
        }
    }
    remember_task_project_id(cache.as_ref(), &task_id, &resolved.project_id);
    print!(
        "{}",
        format_task_action_output(&task_id, &resolved.project_id, "completed", output)?
    );

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
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn task_delete(args: TaskDeleteArgs) -> Result<()> {
    let TaskDeleteArgs {
        task_id,
        project_id,
        list,
        confirm,
        output,
    } = args;
    let client = authenticated_client()?;
    let cache = cache_store();
    let explicit_scope = project_id.is_some() || list.is_some();
    let mut resolved =
        resolve_task_project_id(&client, cache.as_ref(), &task_id, project_id, list).await?;

    if confirm {
        println!("Are you sure you want to delete task '{}'? [y/N]", task_id);
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    if let Err(err) = client.delete_task(&resolved.project_id, &task_id).await {
        if resolved.from_cache && !explicit_scope {
            forget_task_project_id(cache.as_ref(), &task_id);
            resolved =
                resolve_task_project_id(&client, cache.as_ref(), &task_id, None, None).await?;
            client.delete_task(&resolved.project_id, &task_id).await?;
        } else {
            return Err(err);
        }
    }
    forget_task_project_id(cache.as_ref(), &task_id);
    print!(
        "{}",
        format_task_action_output(&task_id, &resolved.project_id, "deleted", output)?
    );

    Ok(())
}

fn format_task_create_output(task: &Task, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(task)?)),
        OutputFormat::Human => Ok(format!(
            "Task created: {}\nID: {}\n",
            task.title,
            task.id.clone().unwrap_or_default()
        )),
    }
}

fn format_task_update_output(task: &Task, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(task)?)),
        OutputFormat::Human => Ok(format!("Task updated: {}\n", task.title)),
    }
}

fn format_task_action_output(
    task_id: &str,
    project_id: &str,
    status: &str,
    format: OutputFormat,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": status,
                "taskId": task_id,
                "projectId": project_id,
            }))?
        )),
        OutputFormat::Human => Ok(format!("Task {}: {}\n", status, task_id)),
    }
}
