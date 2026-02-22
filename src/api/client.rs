use crate::config::Config;
use crate::models::{Column, Project, ProjectData, Task};
use anyhow::{anyhow, Context, Result};
use reqwest::{header, Client, Response};
use serde::Deserialize;
use serde_json::json;

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
    config: Config,
}

impl TickTickClient {
    pub fn new(config: Config) -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client, config })
    }

    async fn request(
        &self,
        method: &str,
        endpoint: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Response> {
        let url = format!("{}{}", BASE_URL, endpoint);
        let mut request = match method {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            "PUT" => self.client.put(&url),
            "DELETE" => self.client.delete(&url),
            _ => return Err(anyhow!("Unsupported HTTP method: {}", method)),
        };

        request = request
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.config.access_token),
            )
            .header(header::CONTENT_TYPE, "application/json");

        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await.context("Failed to send request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Request failed: {} - {}", status, body_text));
        }

        Ok(response)
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
        Ok(data.tasks.unwrap_or_default())
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
}
