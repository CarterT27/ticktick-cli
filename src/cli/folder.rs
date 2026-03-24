use super::bootstrap::authenticated_client;
use crate::output::{print_folders, OutputFormat};
use anyhow::Result;
use clap::{Args, Subcommand};
use uuid::Uuid;

#[derive(Subcommand)]
pub enum FolderCommands {
    Add(FolderAddArgs),
    List(FolderListArgs),
    Update(FolderUpdateArgs),
    Delete(FolderDeleteArgs),
}

#[derive(Args)]
pub struct FolderAddArgs {
    name: String,
    #[arg(long)]
    sort_order: Option<i32>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn folder_add(args: FolderAddArgs) -> Result<()> {
    let client = authenticated_client()?;

    let folder = crate::models::Folder {
        id: Uuid::new_v4().to_string(),
        name: args.name,
        is_owner: Some(true),
        closed: Some(false),
        sort_order: args.sort_order.or(Some(0)),
        ..Default::default()
    };

    let created = client.create_folder(&folder).await?;

    match args.output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&created)?);
        }
        OutputFormat::Human => {
            println!("Folder created: {}", created.name);
            println!("ID: {}", created.id);
        }
    }

    Ok(())
}

#[derive(Args)]
pub struct FolderListArgs {
    #[arg(long)]
    name: Option<String>,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn folder_list(args: FolderListArgs) -> Result<()> {
    let client = authenticated_client()?;

    let mut folders = client.get_folders().await?;

    if let Some(name) = args.name {
        folders.retain(|f| f.name.contains(&name));
    }

    print_folders(&folders, args.output);
    Ok(())
}

#[derive(Args)]
pub struct FolderUpdateArgs {
    folder_id: String,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    sort_order: Option<i32>,
}

pub async fn folder_update(args: FolderUpdateArgs) -> Result<()> {
    let client = authenticated_client()?;

    let folders = client.get_folders().await?;
    let mut folder = folders
        .into_iter()
        .find(|f| f.id == args.folder_id)
        .ok_or_else(|| anyhow::anyhow!("Folder not found: {}", args.folder_id))?;

    if let Some(name) = args.name {
        folder.name = name;
    }
    if let Some(sort_order) = args.sort_order {
        folder.sort_order = Some(sort_order);
    }

    client.update_folder(&folder).await?;
    println!("Folder updated: {}", folder.name);
    Ok(())
}

#[derive(Args)]
pub struct FolderDeleteArgs {
    folder_id: String,
    #[arg(long, default_value = "true")]
    confirm: bool,
}

pub async fn folder_delete(args: FolderDeleteArgs) -> Result<()> {
    let client = authenticated_client()?;

    let folders = client.get_folders().await?;
    let folder = folders
        .into_iter()
        .find(|f| f.id == args.folder_id)
        .ok_or_else(|| anyhow::anyhow!("Folder not found: {}", args.folder_id))?;

    if args.confirm {
        println!(
            "Are you sure you want to delete folder '{}'? [y/N]",
            folder.name
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let modified_time = chrono::Utc::now().to_rfc3339();
    client
        .delete_folder(&args.folder_id, &modified_time)
        .await?;
    println!("Folder deleted: {}", folder.name);
    Ok(())
}
