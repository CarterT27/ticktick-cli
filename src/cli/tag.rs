use super::bootstrap::authenticated_client;
use crate::output::{print_tags, OutputFormat};
use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub enum TagCommands {
    Add(TagAddArgs),
    List(TagListArgs),
    Delete(TagDeleteArgs),
}

#[derive(Args)]
pub struct TagAddArgs {
    tag: String,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn tag_add(args: TagAddArgs) -> Result<()> {
    let _client = authenticated_client()?;

    println!("Note: Tags are added by including them in task titles or using task update");
    println!("Tag example: {}", args.tag);

    match args.output {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({"tag": args.tag}))?
            );
        }
        OutputFormat::Human => {
            println!(
                "To use this tag, add it to a task: tt task add 'Buy groceries #{}'",
                args.tag
            );
        }
    }

    Ok(())
}

#[derive(Args)]
pub struct TagListArgs {
    #[arg(long)]
    contains: Option<String>,
    #[arg(long, default_value = "true")]
    with_counts: bool,
    #[arg(long, default_value = "human")]
    output: OutputFormat,
}

pub async fn tag_list(args: TagListArgs) -> Result<()> {
    let client = authenticated_client()?;

    let mut tags = client.get_tags().await?;

    if let Some(contains) = args.contains {
        tags.retain(|t| t.contains(&contains));
    }

    print_tags(&tags, args.output);
    Ok(())
}

#[derive(Args)]
pub struct TagDeleteArgs {
    tag: String,
    #[arg(long)]
    force: bool,
}

pub async fn tag_delete(args: TagDeleteArgs) -> Result<()> {
    let _client = authenticated_client()?;

    if args.force {
        println!("Tag deletion is not directly supported by the API.");
        println!("To remove a tag, update tasks that use it.");
    } else {
        println!("Tag deletion is not directly supported by the API.");
        println!("To remove a tag, update tasks that use it.");
    }

    Ok(())
}
