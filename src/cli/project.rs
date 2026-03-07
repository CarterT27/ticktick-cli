use crate::api::TickTickClient;
use crate::cache::{get_projects_cached, CacheStore};
use crate::config::AppConfig;
use crate::models::{Project, ProjectData};
use crate::output::{print_projects, OutputFormat};
use anyhow::Result;
use clap::{Args, Subcommand};

fn cache_store() -> Option<CacheStore> {
    CacheStore::new().ok()
}

#[derive(Subcommand)]
pub enum ProjectCommands {
    #[command(alias = "new")]
    Add(ProjectAddArgs),
    #[command(alias = "ls")]
    List(ProjectListArgs),
    Get(ProjectGetArgs),
    Data(ProjectDataArgs),
    #[command(alias = "edit")]
    Update(ProjectUpdateArgs),
    #[command(aliases = ["rm", "del"])]
    Delete(ProjectDeleteArgs),
}

#[derive(Args)]
pub struct ProjectAddArgs {
    name: String,
    #[arg(long)]
    color: Option<String>,
    #[arg(long)]
    view_mode: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    group_id: Option<String>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn project_add(args: ProjectAddArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config.load_authenticated().await?;
    let client = TickTickClient::new(config)?;
    let cache = cache_store();

    let project = build_project_from_add_args(&args);

    let created = client.create_project(&project).await?;
    if let Some(cache) = cache.as_ref() {
        let _ = cache.invalidate_projects();
    }

    print!("{}", format_project_create_output(&created, args.output)?);

    Ok(())
}

#[derive(Args)]
pub struct ProjectListArgs {
    #[arg(long)]
    name: Option<String>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn project_list(args: ProjectListArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config.load_authenticated().await?;
    let client = TickTickClient::new(config)?;
    let cache = cache_store();

    let mut projects = get_projects_cached(&client, cache.as_ref(), false).await?;
    filter_projects_by_name(&mut projects, args.name.as_deref());

    print_projects(&projects, args.output);
    Ok(())
}

#[derive(Args)]
pub struct ProjectGetArgs {
    project_id: String,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn project_get(args: ProjectGetArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config.load_authenticated().await?;
    let client = TickTickClient::new(config)?;

    let project = client.get_project(&args.project_id).await?;
    print!("{}", format_project_detail_output(&project, args.output)?);

    Ok(())
}

#[derive(Args)]
pub struct ProjectDataArgs {
    project_id: String,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn project_data(args: ProjectDataArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config.load_authenticated().await?;
    let client = TickTickClient::new(config)?;

    let data = client.get_project_data(&args.project_id).await?;
    print!("{}", format_project_data_output(&data, args.output)?);

    Ok(())
}

#[derive(Args)]
pub struct ProjectUpdateArgs {
    project_id: String,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    color: Option<String>,
    #[arg(long)]
    view_mode: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    sort_order: Option<i64>,
}

pub async fn project_update(args: ProjectUpdateArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config.load_authenticated().await?;
    let client = TickTickClient::new(config)?;
    let cache = cache_store();

    let mut project = client.get_project(&args.project_id).await?;
    apply_project_update_args(&mut project, &args);

    let updated = client.update_project(&args.project_id, &project).await?;
    if let Some(cache) = cache.as_ref() {
        let _ = cache.invalidate_projects();
    }
    println!("Project updated: {}", updated.name);
    Ok(())
}

#[derive(Args)]
pub struct ProjectDeleteArgs {
    project_id: String,
    #[arg(long, default_value = "true")]
    confirm: bool,
}

pub async fn project_delete(args: ProjectDeleteArgs) -> Result<()> {
    let app_config = AppConfig::new()?;
    let config = app_config.load_authenticated().await?;
    let client = TickTickClient::new(config)?;
    let cache = cache_store();

    if !args.confirm {
        client.delete_project(&args.project_id).await?;
        if let Some(cache) = cache.as_ref() {
            let _ = cache.invalidate_projects();
        }
        println!("Project deleted: {}", args.project_id);
        return Ok(());
    }

    let project = client.get_project(&args.project_id).await?;

    if args.confirm {
        println!(
            "Are you sure you want to delete project '{}'? [y/N]",
            project.name
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    client.delete_project(&args.project_id).await?;
    if let Some(cache) = cache.as_ref() {
        let _ = cache.invalidate_projects();
    }
    println!("Project deleted: {}", project.name);
    Ok(())
}

fn build_project_from_add_args(args: &ProjectAddArgs) -> Project {
    Project {
        id: None,
        name: args.name.clone(),
        color: args.color.clone(),
        view_mode: args.view_mode.clone(),
        kind: args.kind.clone(),
        group_id: args.group_id.clone(),
        ..Default::default()
    }
}

fn filter_projects_by_name(projects: &mut Vec<Project>, name: Option<&str>) {
    if let Some(name) = name {
        projects.retain(|project| project.name.contains(name));
    }
}

fn format_project_create_output(project: &Project, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(project)?)),
        OutputFormat::Human => Ok(format!(
            "Project created: {}\nID: {}\n",
            project.name,
            project.id.clone().unwrap_or_default()
        )),
    }
}

fn format_project_detail_output(project: &Project, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(project)?)),
        OutputFormat::Human => Ok(format!(
            "Project: {}\nID: {}\n",
            project.name,
            project.id.clone().unwrap_or_default()
        )),
    }
}

fn format_project_data_output(data: &ProjectData, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(data)?)),
        OutputFormat::Human => {
            let mut output = format!("Project: {}\n", data.project.name);
            if let Some(tasks) = data.tasks.as_ref() {
                output.push_str(&format!("Tasks: {}\n", tasks.len()));
            }
            if let Some(columns) = data.columns.as_ref() {
                output.push_str(&format!("Columns: {}\n", columns.len()));
            }
            Ok(output)
        }
    }
}

fn apply_project_update_args(project: &mut Project, args: &ProjectUpdateArgs) {
    if let Some(name) = args.name.as_ref() {
        project.name = name.clone();
    }
    if let Some(color) = args.color.as_ref() {
        project.color = Some(color.clone());
    }
    if let Some(view_mode) = args.view_mode.as_ref() {
        project.view_mode = Some(view_mode.clone());
    }
    if let Some(kind) = args.kind.as_ref() {
        project.kind = Some(kind.clone());
    }
    if let Some(sort_order) = args.sort_order {
        project.sort_order = Some(sort_order);
    }

    project.id = Some(args.project_id.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Column, Task};

    fn sample_project() -> Project {
        Project {
            id: Some("project-1".to_string()),
            name: "Inbox".to_string(),
            color: Some("#123456".to_string()),
            view_mode: Some("list".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_project_from_add_args_copies_fields() {
        let args = ProjectAddArgs {
            name: "Work".to_string(),
            color: Some("#ffffff".to_string()),
            view_mode: Some("kanban".to_string()),
            kind: Some("TASK".to_string()),
            group_id: Some("group-1".to_string()),
            output: OutputFormat::Human,
        };

        let project = build_project_from_add_args(&args);
        assert_eq!(project.id, None);
        assert_eq!(project.name, "Work");
        assert_eq!(project.color.as_deref(), Some("#ffffff"));
        assert_eq!(project.view_mode.as_deref(), Some("kanban"));
        assert_eq!(project.kind.as_deref(), Some("TASK"));
        assert_eq!(project.group_id.as_deref(), Some("group-1"));
    }

    #[test]
    fn filter_projects_by_name_only_keeps_matches() {
        let mut projects = vec![
            Project {
                name: "Inbox".to_string(),
                ..Default::default()
            },
            Project {
                name: "Work".to_string(),
                ..Default::default()
            },
        ];

        filter_projects_by_name(&mut projects, Some("Work"));
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Work");
    }

    #[test]
    fn format_project_outputs_match_selected_mode() {
        let project = sample_project();

        let created = format_project_create_output(&project, OutputFormat::Human).unwrap();
        assert!(created.contains("Project created: Inbox"));
        assert!(created.contains("ID: project-1"));

        let detail_json = format_project_detail_output(&project, OutputFormat::Json).unwrap();
        assert!(detail_json.contains("\"name\": \"Inbox\""));
    }

    #[test]
    fn format_project_data_output_counts_tasks_and_columns() {
        let data = ProjectData {
            project: sample_project(),
            tasks: Some(vec![Task {
                title: "One".to_string(),
                ..Default::default()
            }]),
            columns: Some(vec![Column {
                id: "col-1".to_string(),
                project_id: "project-1".to_string(),
                name: "Backlog".to_string(),
                ..Default::default()
            }]),
        };

        let output = format_project_data_output(&data, OutputFormat::Human).unwrap();
        assert!(output.contains("Project: Inbox"));
        assert!(output.contains("Tasks: 1"));
        assert!(output.contains("Columns: 1"));
    }

    #[test]
    fn apply_project_update_args_overrides_selected_fields() {
        let mut project = sample_project();
        let args = ProjectUpdateArgs {
            project_id: "project-99".to_string(),
            name: Some("Renamed".to_string()),
            color: Some("#654321".to_string()),
            view_mode: Some("kanban".to_string()),
            kind: Some("TASK".to_string()),
            sort_order: Some(7),
        };

        apply_project_update_args(&mut project, &args);

        assert_eq!(project.id.as_deref(), Some("project-99"));
        assert_eq!(project.name, "Renamed");
        assert_eq!(project.color.as_deref(), Some("#654321"));
        assert_eq!(project.view_mode.as_deref(), Some("kanban"));
        assert_eq!(project.kind.as_deref(), Some("TASK"));
        assert_eq!(project.sort_order, Some(7));
    }
}
