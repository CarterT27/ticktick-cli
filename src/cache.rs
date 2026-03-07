use crate::api::TickTickClient;
use crate::models::{Project, Task};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PROJECT_CACHE_TTL_SECS: i64 = 15;
const TASK_PROJECT_CACHE_TTL_SECS: i64 = 15 * 60;

#[derive(Debug, Clone)]
pub struct CacheStore {
    cache_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectsCacheFile {
    updated_at: i64,
    projects: Vec<Project>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TaskProjectCacheFile {
    tasks: HashMap<String, TaskProjectCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskProjectCacheEntry {
    project_id: String,
    updated_at: i64,
}

impl CacheStore {
    pub fn new() -> Result<Self> {
        let proj_dirs = ProjectDirs::from("", "", "ticktick-cli")
            .context("Failed to get project directories")?;
        Self::from_dir(proj_dirs.cache_dir().to_path_buf())
    }

    fn from_dir(cache_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;
        Ok(Self { cache_dir })
    }

    pub fn load_projects(&self) -> Result<Option<Vec<Project>>> {
        let path = self.projects_path();
        let Some(cache) = self.read_json::<ProjectsCacheFile>(&path)? else {
            return Ok(None);
        };

        if !is_fresh(cache.updated_at, PROJECT_CACHE_TTL_SECS, unix_timestamp()?) {
            let _ = fs::remove_file(path);
            return Ok(None);
        }

        Ok(Some(cache.projects))
    }

    pub fn save_projects(&self, projects: &[Project]) -> Result<()> {
        let cache = ProjectsCacheFile {
            updated_at: unix_timestamp()?,
            projects: projects.to_vec(),
        };
        self.write_json(&self.projects_path(), &cache)
    }

    pub fn invalidate_projects(&self) -> Result<()> {
        let path = self.projects_path();
        if path.exists() {
            fs::remove_file(path).context("Failed to remove project cache file")?;
        }
        Ok(())
    }

    pub fn clear_all(&self) -> Result<()> {
        for path in [self.projects_path(), self.task_projects_path()] {
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove cache file {}", path.display()))?;
            }
        }
        Ok(())
    }

    pub fn get_task_project_id(&self, task_id: &str) -> Result<Option<String>> {
        let mut cache = self.load_task_project_cache()?;
        let now = unix_timestamp()?;
        let changed = prune_stale_task_entries(&mut cache, now);
        let project_id = cache
            .tasks
            .get(task_id)
            .map(|entry| entry.project_id.clone());

        if changed {
            self.write_task_project_cache(&cache)?;
        }

        Ok(project_id)
    }

    pub fn set_task_project_id(&self, task_id: &str, project_id: &str) -> Result<()> {
        let Some(task_id) = normalize_nonempty(task_id) else {
            return Ok(());
        };
        let Some(project_id) = normalize_nonempty(project_id) else {
            return Ok(());
        };

        let mut cache = self.load_task_project_cache()?;
        let now = unix_timestamp()?;
        prune_stale_task_entries(&mut cache, now);
        cache.tasks.insert(
            task_id,
            TaskProjectCacheEntry {
                project_id,
                updated_at: now,
            },
        );
        self.write_task_project_cache(&cache)
    }

    pub fn remember_tasks(&self, tasks: &[Task], fallback_project_id: Option<&str>) -> Result<()> {
        let fallback_project_id = normalize_optional_nonempty(fallback_project_id);
        let mut cache = self.load_task_project_cache()?;
        let now = unix_timestamp()?;
        prune_stale_task_entries(&mut cache, now);

        for task in tasks {
            let Some(task_id) = task.id.as_deref().and_then(normalize_nonempty) else {
                continue;
            };
            let Some(project_id) = normalize_optional_nonempty(task.project_id.as_deref())
                .or_else(|| fallback_project_id.clone())
            else {
                continue;
            };

            cache.tasks.insert(
                task_id,
                TaskProjectCacheEntry {
                    project_id,
                    updated_at: now,
                },
            );
        }

        self.write_task_project_cache(&cache)
    }

    pub fn remove_task_project_id(&self, task_id: &str) -> Result<()> {
        let Some(task_id) = normalize_nonempty(task_id) else {
            return Ok(());
        };

        let mut cache = self.load_task_project_cache()?;
        if cache.tasks.remove(&task_id).is_some() {
            self.write_task_project_cache(&cache)?;
        }
        Ok(())
    }

    fn load_task_project_cache(&self) -> Result<TaskProjectCacheFile> {
        Ok(self
            .read_json::<TaskProjectCacheFile>(&self.task_projects_path())?
            .unwrap_or_default())
    }

    fn write_task_project_cache(&self, cache: &TaskProjectCacheFile) -> Result<()> {
        self.write_json(&self.task_projects_path(), cache)
    }

    fn projects_path(&self) -> PathBuf {
        self.cache_dir.join("projects.json")
    }

    fn task_projects_path(&self) -> PathBuf {
        self.cache_dir.join("task-projects.json")
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<Option<T>> {
        if !path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read cache file {}", path.display()))?;
        let value = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse cache file {}", path.display()))?;
        Ok(Some(value))
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let contents = serde_json::to_string_pretty(value).context("Failed to serialize cache")?;
        fs::write(path, contents)
            .with_context(|| format!("Failed to write cache file {}", path.display()))?;
        Ok(())
    }
}

pub async fn get_projects_cached(
    client: &TickTickClient,
    cache: Option<&CacheStore>,
    force_refresh: bool,
) -> Result<Vec<Project>> {
    if !force_refresh {
        if let Some(cache) = cache {
            match cache.load_projects() {
                Ok(Some(projects)) => return Ok(projects),
                Ok(None) | Err(_) => {}
            }
        }
    }

    let projects = client.get_projects().await?;
    if let Some(cache) = cache {
        let _ = cache.save_projects(&projects);
    }
    Ok(projects)
}

fn normalize_nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_optional_nonempty(value: Option<&str>) -> Option<String> {
    value.and_then(normalize_nonempty)
}

fn prune_stale_task_entries(cache: &mut TaskProjectCacheFile, now: i64) -> bool {
    let original_len = cache.tasks.len();
    cache
        .tasks
        .retain(|_, entry| is_fresh(entry.updated_at, TASK_PROJECT_CACHE_TTL_SECS, now));
    cache.tasks.len() != original_len
}

fn unix_timestamp() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System time is before UNIX_EPOCH")?
        .as_secs() as i64)
}

fn is_fresh(updated_at: i64, ttl_secs: i64, now: i64) -> bool {
    now - updated_at <= ttl_secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_cache_dir() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "ticktick-cli-cache-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn project_cache_uses_ttl() {
        let cache = CacheStore::from_dir(temp_cache_dir()).unwrap();
        let path = cache.projects_path();
        let payload = ProjectsCacheFile {
            updated_at: unix_timestamp().unwrap() - PROJECT_CACHE_TTL_SECS - 1,
            projects: vec![Project {
                id: Some("p1".to_string()),
                name: "Inbox".to_string(),
                ..Default::default()
            }],
        };
        cache.write_json(&path, &payload).unwrap();

        assert!(cache.load_projects().unwrap().is_none());
    }

    #[test]
    fn remember_tasks_prefers_task_project_id_and_fallback() {
        let cache = CacheStore::from_dir(temp_cache_dir()).unwrap();
        let tasks = vec![
            Task {
                id: Some("task-1".to_string()),
                project_id: Some("project-from-task".to_string()),
                title: "One".to_string(),
                ..Default::default()
            },
            Task {
                id: Some("task-2".to_string()),
                title: "Two".to_string(),
                ..Default::default()
            },
        ];

        cache
            .remember_tasks(&tasks, Some("fallback-project"))
            .unwrap();

        assert_eq!(
            cache.get_task_project_id("task-1").unwrap(),
            Some("project-from-task".to_string())
        );
        assert_eq!(
            cache.get_task_project_id("task-2").unwrap(),
            Some("fallback-project".to_string())
        );
    }

    #[test]
    fn stale_task_project_entries_are_pruned() {
        let cache = CacheStore::from_dir(temp_cache_dir()).unwrap();
        let path = cache.task_projects_path();
        let mut payload = TaskProjectCacheFile::default();
        payload.tasks.insert(
            "stale-task".to_string(),
            TaskProjectCacheEntry {
                project_id: "project-1".to_string(),
                updated_at: unix_timestamp().unwrap() - TASK_PROJECT_CACHE_TTL_SECS - 1,
            },
        );
        cache.write_json(&path, &payload).unwrap();

        assert!(cache.get_task_project_id("stale-task").unwrap().is_none());
    }
}
