use crate::api::TickTickClient;
use crate::config::AppConfig;
use crate::output::{print_projects, OutputFormat};
use anyhow::Result;
use clap::{Args, Subcommand};

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
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let project = crate::models::Project {
        id: None,
        name: args.name,
        color: args.color,
        view_mode: args.view_mode,
        kind: args.kind,
        group_id: args.group_id,
        ..Default::default()
    };

    let created = client.create_project(&project).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&created)?);
        }
        OutputFormat::Human => {
            println!("Project created: {}", created.name);
            println!("ID: {}", created.id.clone().unwrap_or_default());
        }
    }

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
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut projects = client.get_projects().await?;

    if let Some(name) = args.name {
        projects.retain(|p| p.name.contains(&name));
    }

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
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let project = client.get_project(&args.project_id).await?;

    match args.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&project)?),
        OutputFormat::Human => {
            println!("Project: {}", project.name);
            println!("ID: {}", project.id.clone().unwrap_or_default());
        }
    }

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
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let data = client.get_project_data(&args.project_id).await?;

    match args.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&data)?),
        OutputFormat::Human => {
            println!("Project: {}", data.project.name);
            if let Some(tasks) = data.tasks {
                println!("Tasks: {}", tasks.len());
            }
            if let Some(columns) = data.columns {
                println!("Columns: {}", columns.len());
            }
        }
    }

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
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

    let mut project = client.get_project(&args.project_id).await?;

    if let Some(name) = args.name {
        project.name = name;
    }
    if let Some(color) = args.color {
        project.color = Some(color);
    }
    if let Some(view_mode) = args.view_mode {
        project.view_mode = Some(view_mode);
    }
    if let Some(kind) = args.kind {
        project.kind = Some(kind);
    }
    if let Some(sort_order) = args.sort_order {
        project.sort_order = Some(sort_order);
    }

    project.id = Some(args.project_id.clone());

    let updated = client.update_project(&args.project_id, &project).await?;
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
    let config = app_config
        .load()?
        .ok_or_else(|| anyhow::anyhow!("Not authenticated. Run 'tt auth login' first."))?;
    let client = TickTickClient::new(config)?;

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
    println!("Project deleted: {}", project.name);
    Ok(())
}
