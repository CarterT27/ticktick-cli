use crate::config::auth::TickTickOAuth;
use crate::config::{AppConfig, Config};
use crate::models::{Column, Project, ProjectData, Task};
use anyhow::{anyhow, Context, Result};
use reqwest::{header, Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const BASE_URL: &str = "https://api.ticktick.com/open/v1";
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxProjectData {
    #[allow(dead_code)]
    project: Option<Project>,
    tasks: Option<Vec<Task>>,
    #[allow(dead_code)]
    columns: Option<Vec<Column>>,
}

#[derive(Debug, Clone)]
pub struct TickTickClient {
    client: Client,
    config: Arc<Mutex<Config>>,
    app_config: AppConfig,
}

impl TickTickClient {
    pub fn new(config: Config) -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            config: Arc::new(Mutex::new(config)),
            app_config: AppConfig::new()?,
        })
    }

    async fn request(
        &self,
        method: &str,
        endpoint: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Response> {
        validate_http_method(method)?;
        self.refresh_access_token_if_needed().await?;

        let response = self.send_request(method, endpoint, body.as_ref()).await?;
        if should_refresh_after_response(response.status()) {
            self.refresh_access_token().await?;
            let retry_response = self.send_request(method, endpoint, body.as_ref()).await?;
            return response_to_result(retry_response).await;
        }

        response_to_result(response).await
    }

    pub async fn get_projects(&self) -> Result<Vec<Project>> {
        let response = self.request("GET", "/project", None).await?;
        let projects: Vec<Project> = response.json().await.context("Failed to parse response")?;
        Ok(projects)
    }

    pub async fn get_project(&self, project_id: &str) -> Result<Project> {
        let endpoint = format!("/project/{}", project_id);
        let response = self.request("GET", &endpoint, None).await?;
        let project: Project = response.json().await.context("Failed to parse response")?;
        Ok(project)
    }

    pub async fn get_project_data(&self, project_id: &str) -> Result<ProjectData> {
        let endpoint = format!("/project/{}/data", project_id);
        let response = self.request("GET", &endpoint, None).await?;
        let data: ProjectData = response.json().await.context("Failed to parse response")?;
        Ok(data)
    }

    pub async fn get_inbox_tasks(&self) -> Result<Vec<Task>> {
        let response = self.request("GET", "/project/inbox/data", None).await?;
        let data: InboxProjectData = response.json().await.context("Failed to parse response")?;
        Ok(inbox_tasks_from_data(data))
    }

    pub async fn get_project_data_value(&self, project_id: &str) -> Result<serde_json::Value> {
        let endpoint = format!("/project/{}/data", project_id);
        let response = self.request("GET", &endpoint, None).await?;
        let data: serde_json::Value = response.json().await.context("Failed to parse response")?;
        Ok(data)
    }

    pub async fn create_project(&self, project: &Project) -> Result<Project> {
        let body = json!(project);
        let response = self.request("POST", "/project", Some(body)).await?;
        let created: Project = response.json().await.context("Failed to parse response")?;
        Ok(created)
    }

    pub async fn update_project(&self, project_id: &str, project: &Project) -> Result<Project> {
        let endpoint = format!("/project/{}", project_id);
        let body = json!(project);
        let response = self.request("POST", &endpoint, Some(body)).await?;
        let updated: Project = response.json().await.context("Failed to parse response")?;
        Ok(updated)
    }

    pub async fn delete_project(&self, project_id: &str) -> Result<()> {
        let endpoint = format!("/project/{}", project_id);
        self.request("DELETE", &endpoint, None).await?;
        Ok(())
    }

    pub async fn get_task(&self, project_id: &str, task_id: &str) -> Result<Task> {
        let endpoint = format!("/project/{}/task/{}", project_id, task_id);
        let response = self.request("GET", &endpoint, None).await?;
        let task: Task = response.json().await.context("Failed to parse response")?;
        Ok(task)
    }

    pub async fn create_task(&self, task: &Task) -> Result<Task> {
        let body = json!(task);
        let response = self.request("POST", "/task", Some(body)).await?;
        let created: Task = response.json().await.context("Failed to parse response")?;
        Ok(created)
    }

    pub async fn update_task(&self, task_id: &str, task: &Task) -> Result<Task> {
        let endpoint = format!("/task/{}", task_id);
        let body = json!(task);
        let response = self.request("POST", &endpoint, Some(body)).await?;
        let updated: Task = response.json().await.context("Failed to parse response")?;
        Ok(updated)
    }

    pub async fn complete_task(&self, project_id: &str, task_id: &str) -> Result<()> {
        let endpoint = format!("/project/{}/task/{}/complete", project_id, task_id);
        self.request("POST", &endpoint, None).await?;
        Ok(())
    }

    pub async fn delete_task(&self, project_id: &str, task_id: &str) -> Result<()> {
        let endpoint = format!("/project/{}/task/{}", project_id, task_id);
        self.request("DELETE", &endpoint, None).await?;
        Ok(())
    }

    async fn send_request(
        &self,
        method: &str,
        endpoint: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Response> {
        let url = build_url(endpoint);
        let access_token = self.access_token()?;
        let mut request = match method {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            "PUT" => self.client.put(&url),
            "DELETE" => self.client.delete(&url),
            _ => unreachable!("validate_http_method rejects unsupported methods"),
        };

        request = request
            .header(header::AUTHORIZATION, bearer_token_value(&access_token))
            .header(header::CONTENT_TYPE, "application/json");

        if let Some(body) = body {
            request = request.json(body);
        }

        request.send().await.context("Failed to send request")
    }

    async fn refresh_access_token_if_needed(&self) -> Result<()> {
        if self
            .config_snapshot()?
            .is_access_token_expired(current_timestamp()?)
        {
            self.refresh_access_token().await?;
        }

        Ok(())
    }

    async fn refresh_access_token(&self) -> Result<()> {
        let current_config = self.config_snapshot()?;
        if current_config.refresh_token.is_empty() {
            return Err(anyhow!(
                "Access token cannot be refreshed because no refresh token is available. Run 'tt auth login' first."
            ));
        }

        let oauth = oauth_client_from_env()?;
        let refreshed = oauth
            .refresh_access_token(&current_config.refresh_token)
            .await
            .context("Failed to refresh access token")?;

        let mut updated_config = current_config;
        updated_config.update_tokens(
            refreshed.access_token,
            refreshed.refresh_token,
            refreshed.expires_at,
        );

        self.app_config
            .save(&updated_config)
            .context("Failed to persist refreshed credentials")?;

        let mut config = self.lock_config()?;
        *config = updated_config;
        Ok(())
    }

    fn access_token(&self) -> Result<String> {
        Ok(self.lock_config()?.access_token.clone())
    }

    fn config_snapshot(&self) -> Result<Config> {
        Ok(self.lock_config()?.clone())
    }

    fn lock_config(&self) -> Result<std::sync::MutexGuard<'_, Config>> {
        self.config
            .lock()
            .map_err(|_| anyhow!("Authentication state is unavailable"))
    }
}

fn inbox_tasks_from_data(data: InboxProjectData) -> Vec<Task> {
    data.tasks.unwrap_or_default()
}

fn validate_http_method(method: &str) -> Result<()> {
    match method {
        "GET" | "POST" | "PUT" | "DELETE" => Ok(()),
        _ => Err(anyhow!("Unsupported HTTP method: {}", method)),
    }
}

fn build_url(endpoint: &str) -> String {
    format!("{}{}", BASE_URL, endpoint)
}

fn bearer_token_value(access_token: &str) -> String {
    format!("Bearer {}", access_token)
}

fn oauth_client_from_env() -> Result<TickTickOAuth> {
    let client_id =
        std::env::var("TICKTICK_CLIENT_ID").map_err(|_| anyhow!("Missing TICKTICK_CLIENT_ID"))?;
    let client_secret = std::env::var("TICKTICK_CLIENT_SECRET")
        .map_err(|_| anyhow!("Missing TICKTICK_CLIENT_SECRET"))?;
    let redirect_uri = std::env::var("TICKTICK_REDIRECT_URI")
        .unwrap_or_else(|_| "http://localhost:8080/callback".to_string());

    TickTickOAuth::new(client_id, client_secret, redirect_uri)
}

async fn response_to_result(response: Response) -> Result<Response> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();
    Err(anyhow!("Request failed: {} - {}", status, body_text))
}

fn current_timestamp() -> Result<i64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64)
}

fn should_refresh_after_response(status: StatusCode) -> bool {
    status == StatusCode::UNAUTHORIZED
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_http_method_accepts_supported_methods() {
        for method in ["GET", "POST", "PUT", "DELETE"] {
            validate_http_method(method).unwrap();
        }
    }

    #[test]
    fn validate_http_method_rejects_unsupported_methods() {
        let err = validate_http_method("PATCH").unwrap_err().to_string();
        assert!(err.contains("Unsupported HTTP method: PATCH"));
    }

    #[test]
    fn inbox_task_extraction_defaults_to_empty_list() {
        let data: InboxProjectData = serde_json::from_value(json!({})).unwrap();
        assert!(inbox_tasks_from_data(data).is_empty());
    }

    #[test]
    fn inbox_task_extraction_returns_present_tasks() {
        let data: InboxProjectData = serde_json::from_value(json!({
            "tasks": [
                { "id": "task-1", "title": "Follow up" }
            ]
        }))
        .unwrap();

        let tasks = inbox_tasks_from_data(data);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id.as_deref(), Some("task-1"));
        assert_eq!(tasks[0].title, "Follow up");
    }

    #[test]
    fn build_url_joins_base_url_and_endpoint() {
        assert_eq!(
            build_url("/project/inbox/data"),
            "https://api.ticktick.com/open/v1/project/inbox/data"
        );
    }

    #[test]
    fn bearer_token_value_formats_authorization_header() {
        assert_eq!(bearer_token_value("abc123"), "Bearer abc123");
    }

    #[test]
    fn should_refresh_after_response_only_for_401() {
        assert!(should_refresh_after_response(StatusCode::UNAUTHORIZED));
        assert!(!should_refresh_after_response(StatusCode::OK));
        assert!(!should_refresh_after_response(StatusCode::FORBIDDEN));
    }

    #[test]
    fn current_timestamp_returns_unix_seconds() {
        assert!(current_timestamp().unwrap() > 0);
    }
}
