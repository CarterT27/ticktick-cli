use crate::api::TickTickClient;
use crate::cache::{get_projects_cached, CacheStore};
use crate::config::AppConfig;
use crate::models::{Task, TaskStatus};
use crate::output::{print_tasks, OutputFormat};
use anyhow::{anyhow, Result};
use atty::Stream;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc, Weekday};
use clap::{Args, Subcommand};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{self, Read};
use tokio::task::JoinSet;

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

const MAX_CONCURRENT_PROJECT_FETCHES: usize = 8;

#[derive(Debug, Clone)]
struct ResolvedTaskProjectId {
    project_id: String,
    from_cache: bool,
}

fn cache_store() -> Option<CacheStore> {
    CacheStore::new().ok()
}

fn remember_tasks(cache: Option<&CacheStore>, tasks: &[Task], fallback_project_id: Option<&str>) {
    if let Some(cache) = cache {
        let _ = cache.remember_tasks(tasks, fallback_project_id);
    }
}

fn remember_task(cache: Option<&CacheStore>, task: &Task, fallback_project_id: Option<&str>) {
    remember_tasks(cache, std::slice::from_ref(task), fallback_project_id);
}

fn remember_task_project_id(cache: Option<&CacheStore>, task_id: &str, project_id: &str) {
    if let Some(cache) = cache {
        let _ = cache.set_task_project_id(task_id, project_id);
    }
}

fn forget_task_project_id(cache: Option<&CacheStore>, task_id: &str) {
    if let Some(cache) = cache {
        let _ = cache.remove_task_project_id(task_id);
    }
}

fn cached_task_project_id(cache: Option<&CacheStore>, task_id: &str) -> Option<String> {
    cache.and_then(|cache| cache.get_task_project_id(task_id).ok().flatten())
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

fn is_inbox_list_name(value: &str) -> bool {
    value.eq_ignore_ascii_case("inbox") || normalize_list_name(value) == "inbox"
}

fn extract_implicit_list_from_terms(terms: &mut Vec<String>) -> Option<String> {
    if terms.len() == 1 && is_inbox_list_name(&terms[0]) {
        return Some(terms.remove(0));
    }

    None
}

fn normalize_project_id(value: Option<String>) -> Option<String> {
    value.and_then(|id| {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn task_project_id_or_fallback(task: &Task, project_id: &str) -> Option<String> {
    normalize_project_id(task.project_id.clone())
        .or_else(|| normalize_project_id(Some(project_id.to_string())))
}

fn parse_tasks_array(value: &Value) -> Option<Vec<Task>> {
    serde_json::from_value::<Vec<Task>>(value.clone()).ok()
}

fn parse_lossy_tasks_array(value: &Value) -> Option<Vec<Task>> {
    let values = value.as_array()?;
    Some(
        values
            .iter()
            .filter_map(|item| serde_json::from_value::<Task>(item.clone()).ok())
            .collect(),
    )
}

fn extract_inbox_tasks_from_value(value: &Value) -> Option<Vec<Task>> {
    if let Some(tasks) = value.get("tasks").and_then(parse_tasks_array) {
        return Some(tasks);
    }

    for key in ["data", "result"] {
        if let Some(tasks) = value
            .get(key)
            .and_then(|wrapped| wrapped.get("tasks"))
            .and_then(parse_tasks_array)
        {
            return Some(tasks);
        }
    }

    if let Some(tasks) = value.get("task").and_then(|task| {
        serde_json::from_value::<Task>(task.clone())
            .ok()
            .map(|parsed| vec![parsed])
    }) {
        return Some(tasks);
    }

    if let Some(sync) = value.get("syncTaskBean") {
        if let Some(tasks) = sync.get("tasks").and_then(parse_tasks_array) {
            return Some(tasks);
        }

        let mut merged = Vec::new();
        for key in ["update", "add"] {
            if let Some(tasks) = sync.get(key).and_then(parse_lossy_tasks_array) {
                merged.extend(tasks);
            }
        }

        if !merged.is_empty() {
            return Some(merged);
        }
    }

    parse_tasks_array(value)
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

async fn resolve_project_from_list(
    client: &TickTickClient,
    cache: Option<&CacheStore>,
    list_name: &str,
) -> Result<String> {
    let projects = get_projects_cached(client, cache, false).await?;
    let needle = normalize_list_name(list_name);

    let project = projects.iter().find(|p| {
        p.name.eq_ignore_ascii_case(list_name)
            || (!needle.is_empty() && normalize_list_name(&p.name) == needle)
    });

    let Some(project) = project else {
        if is_inbox_list_name(list_name) {
            return Ok(String::new());
        }
        return Err(anyhow!("List not found: {}", list_name));
    };

    if let Some(project_id) = normalize_project_id(project.id.clone()) {
        return Ok(project_id);
    }

    if project.kind.as_deref() == Some("INBOX") || project.name.eq_ignore_ascii_case("inbox") {
        return Ok(String::new());
    }

    Err(anyhow!("List '{}' has no project ID", list_name))
}

async fn resolve_project_id(
    client: &TickTickClient,
    cache: Option<&CacheStore>,
    project_id: Option<String>,
    list_name: Option<String>,
) -> Result<Option<String>> {
    if let Some(project_id) = project_id {
        return Ok(Some(project_id));
    }

    if let Some(list_name) = list_name {
        return Ok(Some(
            resolve_project_from_list(client, cache, &list_name).await?,
        ));
    }

    Ok(None)
}

async fn infer_default_project_id(
    client: &TickTickClient,
    cache: Option<&CacheStore>,
) -> Result<String> {
    let projects = get_projects_cached(client, cache, false).await?;

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
    if project_id.trim().is_empty() {
        if let Ok(tasks) = client.get_inbox_tasks().await {
            return Ok(tasks);
        }

        let mut last_error = None;
        for candidate in ["", "inbox", "INBOX"] {
            match client.get_project_data_value(candidate).await {
                Ok(data) => {
                    if let Some(tasks) = extract_inbox_tasks_from_value(&data) {
                        return Ok(tasks);
                    }

                    let preview = serde_json::to_string(&data)
                        .unwrap_or_else(|_| "<invalid json>".to_string());
                    last_error = Some(anyhow!(
                        "Inbox payload did not include parseable tasks: {}",
                        preview.chars().take(240).collect::<String>()
                    ));
                }
                Err(err) => last_error = Some(err),
            }
        }

        return Err(anyhow!(
            "Unable to fetch Inbox tasks from the API.{}",
            last_error
                .map(|e| format!(" Last error: {}", e))
                .unwrap_or_default()
        ));
    }

    let data = client.get_project_data(project_id).await?;
    Ok(data.tasks.unwrap_or_default())
}

async fn fetch_tasks_for_project_batch(
    client: &TickTickClient,
    project_ids: &[String],
) -> Result<Vec<(String, Vec<Task>)>> {
    let mut results = Vec::with_capacity(project_ids.len());
    let mut tasks = JoinSet::new();

    for (index, project_id) in project_ids.iter().cloned().enumerate() {
        let client = client.clone();
        tasks.spawn(async move {
            let data = client.get_project_data(&project_id).await?;
            Ok::<_, anyhow::Error>((index, project_id, data.tasks.unwrap_or_default()))
        });
    }

    while let Some(result) = tasks.join_next().await {
        let (index, project_id, tasks_for_project) =
            result.map_err(|err| anyhow!("Task fetch worker failed: {}", err))??;
        results.push((index, project_id, tasks_for_project));
    }

    results.sort_by_key(|(index, _, _)| *index);
    Ok(results
        .into_iter()
        .map(|(_, project_id, tasks_for_project)| (project_id, tasks_for_project))
        .collect())
}

async fn get_tasks_across_projects(
    client: &TickTickClient,
    cache: Option<&CacheStore>,
) -> Result<Vec<Task>> {
    let projects = get_projects_cached(client, cache, false).await?;
    let mut tasks = Vec::new();
    let project_ids: Vec<String> = projects
        .into_iter()
        .filter_map(|project| normalize_project_id(project.id))
        .collect();

    for batch in project_ids.chunks(MAX_CONCURRENT_PROJECT_FETCHES) {
        let batch_tasks = fetch_tasks_for_project_batch(client, batch).await?;
        for (project_id, project_tasks) in batch_tasks {
            remember_tasks(cache, &project_tasks, Some(&project_id));
            tasks.extend(project_tasks);
        }
    }

    // TickTick's OpenAPI `/project` can omit Inbox, so fetch it explicitly.
    if let Ok(inbox_tasks) = get_tasks_for_project(client, "").await {
        remember_tasks(cache, &inbox_tasks, None);
        tasks.extend(inbox_tasks);
    }

    dedupe_tasks_by_id(&mut tasks);
    Ok(tasks)
}

fn dedupe_tasks_by_id(tasks: &mut Vec<Task>) {
    let mut seen = HashSet::new();
    tasks.retain(|task| match task.id.as_deref() {
        Some(id) => seen.insert(id.to_string()),
        None => true,
    });
}

async fn resolve_task_project_id(
    client: &TickTickClient,
    cache: Option<&CacheStore>,
    task_id: &str,
    project_id: Option<String>,
    list_name: Option<String>,
) -> Result<ResolvedTaskProjectId> {
    let inbox_requested = list_name
        .as_deref()
        .map(is_inbox_list_name)
        .unwrap_or(false);

    if let Some(project_id) =
        resolve_project_id(client, cache, project_id, list_name.clone()).await?
    {
        if let Some(project_id) = normalize_project_id(Some(project_id)) {
            return Ok(ResolvedTaskProjectId {
                project_id,
                from_cache: false,
            });
        }
    }

    if list_name.is_none() {
        if let Some(project_id) = cached_task_project_id(cache, task_id) {
            return Ok(ResolvedTaskProjectId {
                project_id,
                from_cache: true,
            });
        }
    }

    let projects = get_projects_cached(client, cache, false).await?;
    let project_ids: Vec<String> = projects
        .into_iter()
        .filter_map(|project| normalize_project_id(project.id))
        .collect();
    let mut found_without_project_id = false;

    for batch in project_ids.chunks(MAX_CONCURRENT_PROJECT_FETCHES) {
        let batch_tasks = fetch_tasks_for_project_batch(client, batch).await?;
        for (project_id, tasks_for_project) in batch_tasks {
            remember_tasks(cache, &tasks_for_project, Some(&project_id));
            if let Some(task) = tasks_for_project
                .iter()
                .find(|task| task.id.as_deref() == Some(task_id))
            {
                if let Some(resolved_id) = task_project_id_or_fallback(task, &project_id) {
                    remember_task_project_id(cache, task_id, &resolved_id);
                    return Ok(ResolvedTaskProjectId {
                        project_id: resolved_id,
                        from_cache: false,
                    });
                }
                found_without_project_id = true;
            }
        }
    }

    let inbox_tasks = get_tasks_for_project(client, "").await;
    match inbox_tasks {
        Ok(tasks) => {
            remember_tasks(cache, &tasks, None);
            if let Some(task) = tasks.iter().find(|t| t.id.as_deref() == Some(task_id)) {
                if let Some(resolved_id) = task_project_id_or_fallback(task, "") {
                    remember_task_project_id(cache, task_id, &resolved_id);
                    return Ok(ResolvedTaskProjectId {
                        project_id: resolved_id,
                        from_cache: false,
                    });
                }
                found_without_project_id = true;
            }
        }
        Err(err) => {
            if inbox_requested {
                return Err(err);
            }
        }
    }

    if found_without_project_id {
        return Err(anyhow!(
            "Task '{}' was found, but its list ID is unavailable. Pass a non-empty --project-id.",
            task_id
        ));
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
    let config = app_config.load_authenticated().await?;
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

    let task = crate::models::Task {
        id: None,
        title,
        content: args.content,
        desc: args.desc,
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
    let config = app_config.load_authenticated().await?;
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

    // Convenience alias: treat `tt ls inbox` / `tt ls Inbox` like `--list inbox`.
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
        get_tasks_for_project(&client, &project_id).await?
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
    let config = app_config.load_authenticated().await?;
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
    let config = app_config.load_authenticated().await?;
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
    let config = app_config.load_authenticated().await?;
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
    fn detects_inbox_list_name_variants() {
        assert!(is_inbox_list_name("inbox"));
        assert!(is_inbox_list_name("Inbox"));
        assert!(is_inbox_list_name("  Inbox  "));
        assert!(is_inbox_list_name("📥 Inbox"));
        assert!(!is_inbox_list_name("work"));
    }

    #[test]
    fn extracts_implicit_inbox_list_from_single_term() {
        let mut terms = vec!["inbox".to_string()];
        assert_eq!(
            extract_implicit_list_from_terms(&mut terms),
            Some("inbox".to_string())
        );
        assert!(terms.is_empty());

        let mut terms = vec!["inbox".to_string(), "urgent".to_string()];
        assert_eq!(extract_implicit_list_from_terms(&mut terms), None);
        assert_eq!(terms, vec!["inbox".to_string(), "urgent".to_string()]);
    }

    #[test]
    fn extracts_inbox_tasks_from_multiple_payload_shapes() {
        let direct = serde_json::json!({
            "tasks": [
                {"id": "a", "title": "one", "projectId": "p"}
            ]
        });
        let wrapped = serde_json::json!({
            "data": {
                "tasks": [
                    {"id": "b", "title": "two", "projectId": "p"}
                ]
            }
        });
        let array = serde_json::json!([
            {"id": "c", "title": "three", "projectId": "p"}
        ]);
        let sync = serde_json::json!({
            "syncTaskBean": {
                "update": [
                    {"id": "d", "title": "four", "projectId": "p"}
                ]
            }
        });

        assert_eq!(extract_inbox_tasks_from_value(&direct).unwrap().len(), 1);
        assert_eq!(extract_inbox_tasks_from_value(&wrapped).unwrap().len(), 1);
        assert_eq!(extract_inbox_tasks_from_value(&array).unwrap().len(), 1);
        assert_eq!(extract_inbox_tasks_from_value(&sync).unwrap().len(), 1);
    }

    #[test]
    fn normalizes_project_ids() {
        assert_eq!(normalize_project_id(None), None);
        assert_eq!(normalize_project_id(Some("".to_string())), None);
        assert_eq!(normalize_project_id(Some("   ".to_string())), None);
        assert_eq!(
            normalize_project_id(Some("  abc123  ".to_string())),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn task_project_id_prefers_task_and_falls_back_to_container() {
        let mut task = Task {
            title: "sample".to_string(),
            ..Default::default()
        };
        task.project_id = Some("real-project".to_string());
        assert_eq!(
            task_project_id_or_fallback(&task, ""),
            Some("real-project".to_string())
        );

        task.project_id = None;
        assert_eq!(
            task_project_id_or_fallback(&task, "container-project"),
            Some("container-project".to_string())
        );

        assert_eq!(task_project_id_or_fallback(&task, "  "), None);
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
