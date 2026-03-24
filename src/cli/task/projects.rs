use super::filters::{is_inbox_list_name, normalize_list_name};
use crate::api::TickTickClient;
use crate::cache::{get_projects_cached, CacheStore};
use crate::models::Task;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::HashSet;
use tokio::task::JoinSet;

const MAX_CONCURRENT_PROJECT_FETCHES: usize = 8;

#[derive(Debug, Clone)]
pub(super) struct ResolvedTaskProjectId {
    pub(super) project_id: String,
    pub(super) from_cache: bool,
}

pub(super) fn cache_store() -> Option<CacheStore> {
    CacheStore::new().ok()
}

pub(super) fn remember_tasks(
    cache: Option<&CacheStore>,
    tasks: &[Task],
    fallback_project_id: Option<&str>,
) {
    if let Some(cache) = cache {
        let _ = cache.remember_tasks(tasks, fallback_project_id);
    }
}

pub(super) fn remember_task(
    cache: Option<&CacheStore>,
    task: &Task,
    fallback_project_id: Option<&str>,
) {
    remember_tasks(cache, std::slice::from_ref(task), fallback_project_id);
}

fn store_task_project_id(cache: Option<&CacheStore>, task_id: &str, project_id: &str) {
    if let Some(cache) = cache {
        let _ = cache.set_task_project_id(task_id, project_id);
    }
}

pub(super) fn forget_task_project_id(cache: Option<&CacheStore>, task_id: &str) {
    if let Some(cache) = cache {
        let _ = cache.remove_task_project_id(task_id);
    }
}

fn cached_task_project_id(cache: Option<&CacheStore>, task_id: &str) -> Option<String> {
    cache.and_then(|cache| cache.get_task_project_id(task_id).ok().flatten())
}

pub(super) fn remember_task_project_id(
    cache: Option<&CacheStore>,
    task_id: &str,
    project_id: &str,
) {
    store_task_project_id(cache, task_id, project_id);
}

async fn resolve_project_from_list(
    client: &TickTickClient,
    cache: Option<&CacheStore>,
    list_name: &str,
) -> Result<String> {
    let projects = get_projects_cached(client, cache, false).await?;
    let needle = normalize_list_name(list_name);

    let project = projects.iter().find(|project| {
        project.name.eq_ignore_ascii_case(list_name)
            || (!needle.is_empty() && normalize_list_name(&project.name) == needle)
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

pub(super) async fn resolve_project_id(
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

pub(super) async fn infer_default_project_id(
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
        .find(|project| project.kind.as_deref() == Some("INBOX"))
        .or_else(|| {
            projects
                .iter()
                .find(|project| project.name.eq_ignore_ascii_case("inbox"))
        })
        .or_else(|| {
            projects
                .iter()
                .find(|project| !project.closed.unwrap_or(false))
        })
        .or_else(|| projects.first());

    default
        .and_then(|project| project.id.clone())
        .ok_or_else(|| anyhow!("Unable to infer a default list. Pass --project-id or --list."))
}

pub(super) async fn get_tasks_for_project(
    client: &TickTickClient,
    project_id: &str,
) -> Result<Vec<Task>> {
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
                .map(|err| format!(" Last error: {}", err))
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

pub(super) async fn get_tasks_across_projects(
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

pub(super) async fn resolve_task_project_id(
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
                    store_task_project_id(cache, task_id, &resolved_id);
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
            if let Some(task) = tasks
                .iter()
                .find(|task| task.id.as_deref() == Some(task_id))
            {
                if let Some(resolved_id) = task_project_id_or_fallback(task, "") {
                    store_task_project_id(cache, task_id, &resolved_id);
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

pub(super) fn normalize_project_id(value: Option<String>) -> Option<String> {
    value.and_then(|id| {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(super) fn task_project_id_or_fallback(task: &Task, project_id: &str) -> Option<String> {
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

pub(super) fn extract_inbox_tasks_from_value(value: &Value) -> Option<Vec<Task>> {
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
