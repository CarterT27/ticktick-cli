use crate::api::TickTickClient;
use crate::config::AppConfig;
use crate::models::{Task, TaskStatus};
use crate::output::{print_tasks, OutputFormat};
use anyhow::{anyhow, Result};
use atty::Stream;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc, Weekday};
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

#[derive(Default)]
struct ShorthandFilters {
    priority: Option<i32>,
    list: Option<String>,
    tags: Vec<String>,
    when: Option<TaskWhenFilter>,
    terms: Vec<String>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskWhenFilter {
    Today,
    Tomorrow,
    #[value(alias = "thisweek", alias = "this-week", alias = "week")]
    ThisWeek,
}

fn parse_priority_shorthand(token: &str) -> Option<i32> {
    let value = token.strip_prefix('!')?.to_ascii_lowercase();
    match value.as_str() {
        "high" => Some(5),
        "medium" => Some(3),
        "low" => Some(1),
        "none" | "normal" => Some(0),
        _ => None,
    }
}

fn parse_when_token(token: &str) -> Option<TaskWhenFilter> {
    match token.to_ascii_lowercase().as_str() {
        "today" => Some(TaskWhenFilter::Today),
        "tomorrow" => Some(TaskWhenFilter::Tomorrow),
        "week" | "thisweek" | "this-week" => Some(TaskWhenFilter::ThisWeek),
        _ => None,
    }
}

fn parse_shorthand_with_when(raw: &str, parse_when: bool) -> ShorthandFilters {
    let mut parsed = ShorthandFilters::default();
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let mut i = 0;

    while i < tokens.len() {
        let token = tokens[i];
        if let Some(priority) = parse_priority_shorthand(token) {
            parsed.priority = Some(priority);
            i += 1;
            continue;
        }

        if let Some(list) = token.strip_prefix('~') {
            if !list.is_empty() {
                parsed.list = Some(list.to_string());
                i += 1;
                continue;
            }
        }

        if let Some(tag) = token.strip_prefix('#') {
            if !tag.is_empty() {
                parsed.tags.push(tag.to_string());
                i += 1;
                continue;
            }
        }

        if parse_when {
            if token.eq_ignore_ascii_case("this")
                && i + 1 < tokens.len()
                && tokens[i + 1].eq_ignore_ascii_case("week")
            {
                parsed.when = Some(TaskWhenFilter::ThisWeek);
                i += 2;
                continue;
            }

            if let Some(when) = parse_when_token(token) {
                parsed.when = Some(when);
                i += 1;
                continue;
            }
        }

        parsed.terms.push(token.to_string());
        i += 1;
    }

    parsed
}

fn parse_shorthand(raw: &str) -> ShorthandFilters {
    parse_shorthand_with_when(raw, true)
}

fn parse_task_add_shorthand(raw: &str) -> ShorthandFilters {
    parse_shorthand_with_when(raw, false)
}

fn normalize_date_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '/' && ch != '-')
        .to_ascii_lowercase()
}

fn infer_year_for_month_day(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if this_year >= today {
        Some(this_year)
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, month, day)
    }
}

fn parse_year_token(token: &str) -> Option<i32> {
    let year = token.parse::<i32>().ok()?;
    match token.len() {
        2 => Some(2000 + year),
        4 => Some(year),
        _ => None,
    }
}

fn parse_day_token(token: &str) -> Option<u32> {
    let day_text = token
        .strip_suffix("st")
        .or_else(|| token.strip_suffix("nd"))
        .or_else(|| token.strip_suffix("rd"))
        .or_else(|| token.strip_suffix("th"))
        .unwrap_or(token);

    let day = day_text.parse::<u32>().ok()?;
    if (1..=31).contains(&day) {
        Some(day)
    } else {
        None
    }
}

fn parse_month_token(token: &str) -> Option<u32> {
    match token {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_weekday_token(token: &str) -> Option<Weekday> {
    match token {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tues" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thurs" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn next_or_same_weekday(today: NaiveDate, target: Weekday) -> NaiveDate {
    let today_idx = today.weekday().num_days_from_monday() as i64;
    let target_idx = target.num_days_from_monday() as i64;
    let offset = (target_idx - today_idx + 7) % 7;
    today + Duration::days(offset)
}

fn start_of_next_week(today: NaiveDate) -> NaiveDate {
    let start_of_this_week = today - Duration::days(today.weekday().num_days_from_monday().into());
    start_of_this_week + Duration::days(7)
}

fn parse_numeric_date_token(token: &str, today: NaiveDate) -> Option<NaiveDate> {
    if let Ok(date) = NaiveDate::parse_from_str(token, "%Y-%m-%d") {
        return Some(date);
    }

    let separator = if token.contains('/') {
        Some('/')
    } else if token.matches('-').count() == 2 {
        Some('-')
    } else {
        None
    }?;

    let parts: Vec<&str> = token.split(separator).collect();
    if parts.len() == 2 {
        let month = parts[0].parse::<u32>().ok()?;
        let day = parts[1].parse::<u32>().ok()?;
        return infer_year_for_month_day(month, day, today);
    }

    if parts.len() == 3 {
        let month = parts[0].parse::<u32>().ok()?;
        let day = parts[1].parse::<u32>().ok()?;
        let year = parse_year_token(parts[2])?;
        return NaiveDate::from_ymd_opt(year, month, day);
    }

    None
}

fn parse_month_day_sequence(
    tokens: &[&str],
    index: usize,
    today: NaiveDate,
) -> Option<(usize, NaiveDate)> {
    let month = parse_month_token(&normalize_date_token(tokens.get(index)?))?;
    let second = normalize_date_token(tokens.get(index + 1)?);

    // Support "jan 2029" / "january 2029" by mapping to the first of that month.
    if let Some(year) = parse_year_token(&second) {
        let date = NaiveDate::from_ymd_opt(year, month, 1)?;
        return Some((2, date));
    }

    let day = parse_day_token(&second)?;

    if let Some(year_token) = tokens.get(index + 2) {
        let normalized_year = normalize_date_token(year_token);
        if let Some(year) = parse_year_token(&normalized_year) {
            let date = NaiveDate::from_ymd_opt(year, month, day)?;
            return Some((3, date));
        }
    }

    let date = infer_year_for_month_day(month, day, today)?;
    Some((2, date))
}

fn extract_due_date_from_input(raw: &str, today: NaiveDate) -> (String, Option<NaiveDate>) {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    if tokens.is_empty() {
        return (String::new(), None);
    }

    for (index, token) in tokens.iter().enumerate() {
        if token.starts_with('#') || token.starts_with('~') || token.starts_with('!') {
            continue;
        }

        let normalized = normalize_date_token(token);
        if normalized.is_empty() {
            continue;
        }

        if normalized == "next"
            && index + 1 < tokens.len()
            && normalize_date_token(tokens[index + 1]) == "week"
        {
            let date = start_of_next_week(today);
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| {
                    if i == index || i == index + 1 {
                        None
                    } else {
                        Some(*value)
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }

        if let Some((consumed, date)) = parse_month_day_sequence(&tokens, index, today) {
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| {
                    if i >= index && i < index + consumed {
                        None
                    } else {
                        Some(*value)
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }

        if let Some(date) = parse_numeric_date_token(&normalized, today) {
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| if i == index { None } else { Some(*value) })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }

        let relative_date = match normalized.as_str() {
            "today" => Some(today),
            "tomorrow" => Some(today + Duration::days(1)),
            _ => {
                parse_weekday_token(&normalized).map(|weekday| next_or_same_weekday(today, weekday))
            }
        };

        if let Some(date) = relative_date {
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| if i == index { None } else { Some(*value) })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }
    }

    (raw.trim().to_string(), None)
}

fn format_ticktick_due_date(date: NaiveDate) -> Option<String> {
    let local_midnight = date.and_hms_opt(0, 0, 0)?;
    let local_dt = Local
        .from_local_datetime(&local_midnight)
        .earliest()
        .or_else(|| Local.from_local_datetime(&local_midnight).latest())?;
    let utc_dt = local_dt.with_timezone(&Utc);
    Some(utc_dt.format("%Y-%m-%dT%H:%M:%S%.3f+0000").to_string())
}

fn merge_tags(existing: &mut Vec<String>, extras: Vec<String>) {
    for tag in extras {
        if !existing.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            existing.push(tag);
        }
    }
}

fn task_has_all_tags(task: &Task, required_tags: &[String]) -> bool {
    let Some(task_tags) = task.tags.as_ref() else {
        return false;
    };

    required_tags.iter().all(|required| {
        task_tags
            .iter()
            .any(|actual| actual.eq_ignore_ascii_case(required))
    })
}

fn normalize_list_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_task_date(value: &str) -> Option<NaiveDate> {
    if let Ok(epoch) = value.parse::<i64>() {
        let dt = if value.len() > 10 {
            DateTime::<Utc>::from_timestamp_millis(epoch)?
        } else {
            DateTime::<Utc>::from_timestamp(epoch, 0)?
        };
        return Some(dt.date_naive());
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Some(dt.date_naive());
    }

    if let Ok(dt) = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(dt.date_naive());
    }

    if let Ok(dt) = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(dt.date_naive());
    }

    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date);
    }

    let prefix = value.get(0..10)?;
    NaiveDate::parse_from_str(prefix, "%Y-%m-%d").ok()
}

fn task_due_date(task: &Task) -> Option<NaiveDate> {
    task.due_date
        .as_deref()
        .or(task.start_date.as_deref())
        .and_then(parse_task_date)
}

fn date_window_for(when: TaskWhenFilter, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match when {
        TaskWhenFilter::Today => (today, today),
        TaskWhenFilter::Tomorrow => {
            let day = today + Duration::days(1);
            (day, day)
        }
        TaskWhenFilter::ThisWeek => {
            let start = today - Duration::days(today.weekday().num_days_from_monday().into());
            let end = start + Duration::days(6);
            (start, end)
        }
    }
}

fn task_matches_when_filter(task: &Task, when: TaskWhenFilter, today: NaiveDate) -> bool {
    let Some(task_date) = task_due_date(task) else {
        return false;
    };

    let (start, end) = date_window_for(when, today);
    task_date >= start && task_date <= end
}

async fn resolve_project_from_list(client: &TickTickClient, list_name: &str) -> Result<String> {
    let projects = client.get_projects().await?;
    let needle = normalize_list_name(list_name);

    let project = projects
        .iter()
        .find(|p| {
            p.name.eq_ignore_ascii_case(list_name)
                || (!needle.is_empty() && normalize_list_name(&p.name) == needle)
        })
        .ok_or_else(|| anyhow!("List not found: {}", list_name))?;

    project
        .id
        .clone()
        .ok_or_else(|| anyhow!("List '{}' has no project ID", list_name))
}

async fn resolve_project_id(
    client: &TickTickClient,
    project_id: Option<String>,
    list_name: Option<String>,
) -> Result<Option<String>> {
    if let Some(project_id) = project_id {
        return Ok(Some(project_id));
    }

    if let Some(list_name) = list_name {
        return Ok(Some(resolve_project_from_list(client, &list_name).await?));
    }

    Ok(None)
}

async fn infer_default_project_id(client: &TickTickClient) -> Result<String> {
    let projects = client.get_projects().await?;

    if projects.is_empty() {
        return Err(anyhow!(
            "No lists found. Create one with 'tt project add <name>' first."
        ));
    }

    let default = projects
        .iter()
        .find(|p| p.kind.as_deref() == Some("INBOX"))
        .or_else(|| {
            projects
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case("inbox"))
        })
        .or_else(|| projects.iter().find(|p| !p.closed.unwrap_or(false)))
        .or_else(|| projects.first());

    default
        .and_then(|p| p.id.clone())
        .ok_or_else(|| anyhow!("Unable to infer a default list. Pass --project-id or --list."))
}

async fn get_tasks_for_project(client: &TickTickClient, project_id: &str) -> Result<Vec<Task>> {
    let data = client.get_project_data(project_id).await?;
    Ok(data.tasks.unwrap_or_default())
}

async fn get_tasks_across_projects(client: &TickTickClient) -> Result<Vec<Task>> {
    let projects = client.get_projects().await?;
    let mut tasks = Vec::new();

    for project in projects {
        let Some(project_id) = project.id else {
            continue;
        };
        tasks.extend(get_tasks_for_project(client, &project_id).await?);
    }

    Ok(tasks)
}

async fn resolve_task_project_id(
    client: &TickTickClient,
    task_id: &str,
    project_id: Option<String>,
    list_name: Option<String>,
) -> Result<String> {
    if let Some(project_id) = resolve_project_id(client, project_id, list_name).await? {
        return Ok(project_id);
    }

    let projects = client.get_projects().await?;

    for project in projects {
        let Some(project_id) = project.id else {
            continue;
        };

        let data = client.get_project_data(&project_id).await?;
        let found = data
            .tasks
            .unwrap_or_default()
            .iter()
            .any(|t| t.id.as_deref() == Some(task_id));

        if found {
            return Ok(project_id);
        }
    }

    Err(anyhow!(
        "Task '{}' was not found in accessible lists. Pass --project-id or --list.",
        task_id
    ))
}

#[derive(Args)]
pub struct TaskAddArgs {
    title: Vec<String>,
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

    let project_id = match resolve_project_id(&client, args.project_id, args.list).await? {
        Some(project_id) => project_id,
        None => infer_default_project_id(&client).await?,
    };

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
    let search_terms = shorthand.terms;

    let project_id = resolve_project_id(&client, args.project_id, args.list).await?;
    let mut tasks = if let Some(project_id) = project_id {
        get_tasks_for_project(&client, &project_id).await?
    } else {
        get_tasks_across_projects(&client).await?
    };

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

        tasks.retain(|t| {
            let completed = matches!(t.status, Some(TaskStatus::Completed));
            if is_done {
                completed
            } else {
                !completed
            }
        });
    }

    if let Some(prio) = args.priority {
        tasks.retain(|t| t.priority.unwrap_or(0) == prio);
    }

    if !args.tags.is_empty() {
        tasks.retain(|t| task_has_all_tags(t, &args.tags));
    }

    if let Some(when) = args.when {
        let today = Local::now().date_naive();
        tasks.retain(|task| task_matches_when_filter(task, when, today));
    }

    if !search_terms.is_empty() {
        let needles: Vec<String> = search_terms
            .into_iter()
            .map(|s| s.to_ascii_lowercase())
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

    let project_id = resolve_task_project_id(
        &client,
        &args.task_id,
        args.project_id.clone(),
        args.list.clone(),
    )
    .await?;

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

    let project_id =
        resolve_task_project_id(&client, &args.task_id, args.project_id, args.list).await?;

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

    let project_id =
        resolve_task_project_id(&client, &args.task_id, args.project_id, args.list).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(
        due_date: Option<&str>,
        start_date: Option<&str>,
        tags: Option<Vec<&str>>,
        priority: Option<i32>,
    ) -> Task {
        Task {
            title: "sample".to_string(),
            due_date: due_date.map(ToString::to_string),
            start_date: start_date.map(ToString::to_string),
            tags: tags.map(|v| v.into_iter().map(ToString::to_string).collect()),
            priority,
            ..Default::default()
        }
    }

    #[test]
    fn parses_priority_shorthand_case_insensitive() {
        assert_eq!(parse_priority_shorthand("!high"), Some(5));
        assert_eq!(parse_priority_shorthand("!High"), Some(5));
        assert_eq!(parse_priority_shorthand("!medium"), Some(3));
        assert_eq!(parse_priority_shorthand("!Low"), Some(1));
        assert_eq!(parse_priority_shorthand("!none"), Some(0));
        assert_eq!(parse_priority_shorthand("!urgent"), None);
    }

    #[test]
    fn parses_when_tokens() {
        assert_eq!(parse_when_token("today"), Some(TaskWhenFilter::Today));
        assert_eq!(parse_when_token("tomorrow"), Some(TaskWhenFilter::Tomorrow));
        assert_eq!(parse_when_token("week"), Some(TaskWhenFilter::ThisWeek));
        assert_eq!(
            parse_when_token("this-week"),
            Some(TaskWhenFilter::ThisWeek)
        );
        assert_eq!(parse_when_token("other"), None);
    }

    #[test]
    fn parses_shorthand_markers_and_terms() {
        let parsed = parse_shorthand("finish report !High ~Personal #work #ops today");
        assert_eq!(parsed.priority, Some(5));
        assert_eq!(parsed.list.as_deref(), Some("Personal"));
        assert_eq!(parsed.when, Some(TaskWhenFilter::Today));
        assert_eq!(parsed.tags, vec!["work".to_string(), "ops".to_string()]);
        assert_eq!(
            parsed.terms,
            vec!["finish".to_string(), "report".to_string()]
        );
    }

    #[test]
    fn parses_shorthand_this_week_phrase() {
        let parsed = parse_shorthand("plan this week");
        assert_eq!(parsed.when, Some(TaskWhenFilter::ThisWeek));
        assert_eq!(parsed.terms, vec!["plan".to_string()]);
    }

    #[test]
    fn add_shorthand_keeps_when_terms_for_title() {
        let parsed = parse_task_add_shorthand("plan today");
        assert_eq!(parsed.when, None);
        assert_eq!(parsed.terms, vec!["plan".to_string(), "today".to_string()]);
    }

    #[test]
    fn extracts_due_date_today_and_cleans_title() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("finish report today", today);
        assert_eq!(title, "finish report");
        assert_eq!(date, Some(today));
    }

    #[test]
    fn extracts_due_date_next_week_phrase() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("plan roadmap next week", today);
        assert_eq!(title, "plan roadmap");
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 2, 23).unwrap()));
    }

    #[test]
    fn extracts_due_date_weekday() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
        let (title, date) = extract_due_date_from_input("ship draft friday", today);
        assert_eq!(title, "ship draft");
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 2, 20).unwrap()));
    }

    #[test]
    fn extracts_due_date_numeric_month_day() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("pay rent 6/01", today);
        assert_eq!(title, "pay rent");
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()));
    }

    #[test]
    fn extracts_due_date_text_month_day_year() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("renew passport feb 1 2027", today);
        assert_eq!(title, "renew passport");
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2027, 2, 1).unwrap()));
    }

    #[test]
    fn keeps_hashtag_dates_as_tags() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("sync with team #friday", today);
        assert_eq!(title, "sync with team #friday");
        assert_eq!(date, None);
    }

    #[test]
    fn extracts_due_date_text_month_year_short_name() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("plan launch jan 2029", today);
        assert_eq!(title, "plan launch");
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2029, 1, 1).unwrap()));
    }

    #[test]
    fn extracts_due_date_text_month_year_full_name() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("plan launch january 2029", today);
        assert_eq!(title, "plan launch");
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2029, 1, 1).unwrap()));
    }

    #[test]
    fn extracts_due_date_text_month_day_year_capitalized() {
        let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let (title, date) = extract_due_date_from_input("book trip January 3 2028", today);
        assert_eq!(title, "book trip");
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2028, 1, 3).unwrap()));
    }

    #[test]
    fn formats_inferred_due_date_for_ticktick_api() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let value = format_ticktick_due_date(date).unwrap();
        assert!(DateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f%z").is_ok());
        assert!(value.ends_with("+0000"));
    }

    #[test]
    fn merges_tags_without_case_duplicates() {
        let mut tags = vec!["work".to_string()];
        merge_tags(&mut tags, vec!["Work".to_string(), "ops".to_string()]);
        assert_eq!(tags, vec!["work".to_string(), "ops".to_string()]);
    }

    #[test]
    fn matches_tags_case_insensitively() {
        let task = make_task(None, None, Some(vec!["Work", "ops"]), None);
        assert!(task_has_all_tags(
            &task,
            &["work".to_string(), "OPS".to_string()]
        ));
        assert!(!task_has_all_tags(&task, &["missing".to_string()]));
    }

    #[test]
    fn normalizes_list_names_without_emoji() {
        assert_eq!(normalize_list_name("🚀Personal"), "personal");
        assert_eq!(normalize_list_name("👨🏻‍💻 Projects"), "projects");
        assert_eq!(normalize_list_name("Personal Team"), "personal team");
    }

    #[test]
    fn parses_task_date_from_iso_and_prefix() {
        assert_eq!(
            parse_task_date("2026-03-01T00:00:00.000+0000"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap())
        );
        assert_eq!(
            parse_task_date("2026-03-01T00:00:00"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap())
        );
        assert_eq!(
            parse_task_date("2026-03-01"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap())
        );
    }

    #[test]
    fn parses_task_date_from_epoch_values() {
        assert_eq!(
            parse_task_date("1704067200000"),
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        );
        assert_eq!(
            parse_task_date("1704067200"),
            Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        );
    }

    #[test]
    fn computes_date_windows() {
        let base = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        assert_eq!(date_window_for(TaskWhenFilter::Today, base), (base, base));
        assert_eq!(
            date_window_for(TaskWhenFilter::Tomorrow, base),
            (
                NaiveDate::from_ymd_opt(2026, 2, 21).unwrap(),
                NaiveDate::from_ymd_opt(2026, 2, 21).unwrap()
            )
        );
        assert_eq!(
            date_window_for(TaskWhenFilter::ThisWeek, base),
            (
                NaiveDate::from_ymd_opt(2026, 2, 16).unwrap(),
                NaiveDate::from_ymd_opt(2026, 2, 22).unwrap()
            )
        );
    }

    #[test]
    fn filters_tasks_for_when() {
        let base = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
        let today = make_task(Some("2026-02-20"), None, None, None);
        let tomorrow = make_task(Some("2026-02-21"), None, None, None);
        let this_week = make_task(Some("2026-02-22"), None, None, None);
        let next_week = make_task(Some("2026-02-23"), None, None, None);
        let no_date = make_task(None, None, None, None);

        assert!(task_matches_when_filter(
            &today,
            TaskWhenFilter::Today,
            base
        ));
        assert!(!task_matches_when_filter(
            &tomorrow,
            TaskWhenFilter::Today,
            base
        ));
        assert!(task_matches_when_filter(
            &tomorrow,
            TaskWhenFilter::Tomorrow,
            base
        ));
        assert!(task_matches_when_filter(
            &this_week,
            TaskWhenFilter::ThisWeek,
            base
        ));
        assert!(!task_matches_when_filter(
            &next_week,
            TaskWhenFilter::ThisWeek,
            base
        ));
        assert!(!task_matches_when_filter(
            &no_date,
            TaskWhenFilter::Today,
            base
        ));
    }

    #[test]
    fn uses_due_date_then_start_date() {
        let task = make_task(None, Some("2026-03-02"), None, None);
        assert_eq!(
            task_due_date(&task),
            Some(NaiveDate::from_ymd_opt(2026, 3, 2).unwrap())
        );
    }

    #[test]
    fn parses_query_with_unknown_bang_as_term() {
        let parsed = parse_shorthand("review !urgent");
        assert_eq!(parsed.priority, None);
        assert_eq!(
            parsed.terms,
            vec!["review".to_string(), "!urgent".to_string()]
        );
    }

    #[test]
    fn parse_task_date_rejects_invalid_values() {
        assert_eq!(parse_task_date(""), None);
        assert_eq!(parse_task_date("not-a-date"), None);
    }

    #[test]
    fn make_task_helper_sets_priority() {
        let task = make_task(Some("2026-03-01"), None, None, Some(3));
        assert_eq!(task.priority, Some(3));
    }
}
