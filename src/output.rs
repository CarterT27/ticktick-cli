use crate::models::{Project, Task};
use atty::Stream;
use std::io;

#[derive(clap::ValueEnum, Clone, Debug, Default)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

trait Tabular {
    fn headers() -> Vec<String>;
    fn rows(&self) -> Vec<String>;
}

impl Tabular for Task {
    fn headers() -> Vec<String> {
        vec![
            "ID".to_string(),
            "Title".to_string(),
            "Priority".to_string(),
            "Due".to_string(),
        ]
    }

    fn rows(&self) -> Vec<String> {
        let priority = match self.priority.unwrap_or(0) {
            0 => "".to_string(),
            1 => "Low".to_string(),
            3 => "Medium".to_string(),
            5 => "High".to_string(),
            p => p.to_string(),
        };
        let due = self
            .due_date
            .as_ref()
            .map(|d| d.split('T').next().unwrap_or(d).to_string())
            .unwrap_or_default();
        let id = self.id.clone().unwrap_or_default();

        vec![id, self.title.clone(), priority, due]
    }
}

impl Tabular for Project {
    fn headers() -> Vec<String> {
        vec![
            "ID".to_string(),
            "Name".to_string(),
            "Color".to_string(),
            "View".to_string(),
        ]
    }

    fn rows(&self) -> Vec<String> {
        let id = self.id.clone().unwrap_or_default();
        vec![
            format!("{}...", &id[..8.min(id.len())]),
            self.name.clone(),
            self.color.clone().unwrap_or_default(),
            self.view_mode.clone().unwrap_or_default(),
        ]
    }
}

fn print_table<T: Tabular>(items: &[T]) {
    if items.is_empty() {
        println!("No items found.");
        return;
    }

    let headers = T::headers();
    let rows: Vec<Vec<String>> = items.iter().map(|i| i.rows()).collect();

    let col_widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            let max_width = rows
                .iter()
                .map(|row| row.get(i).map_or(0, |c| c.len()))
                .max()
                .unwrap_or(0);
            header.len().max(max_width)
        })
        .collect();

    let separator: String = col_widths
        .iter()
        .map(|w| "-".repeat(*w + 2))
        .collect::<Vec<_>>()
        .join("+");
    let header_row: String = col_widths
        .iter()
        .enumerate()
        .map(|(i, w)| {
            format!(
                " {:width$} ",
                headers.get(i).unwrap_or(&String::new()),
                width = *w
            )
        })
        .collect::<Vec<_>>()
        .join("|");

    println!("|{}|", header_row);
    println!("|{}|", separator);

    for row in rows {
        let row_str: String = col_widths
            .iter()
            .enumerate()
            .map(|(i, w)| {
                format!(
                    " {:width$} ",
                    row.get(i).unwrap_or(&String::new()),
                    width = *w
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        println!("|{}|", row_str);
    }
}

pub fn print_tasks(tasks: &[Task], format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            let _ = serde_json::to_writer_pretty(io::stdout(), &tasks);
            println!();
        }
        OutputFormat::Human => {
            if atty::is(Stream::Stdout) {
                print_table(tasks);
            } else {
                for task in tasks {
                    let id = task.id.clone().unwrap_or_default();
                    println!("{}|{}", id, task.title);
                }
            }
        }
    }
}

pub fn print_projects(projects: &[Project], format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            let _ = serde_json::to_writer_pretty(io::stdout(), &projects);
            println!();
        }
        OutputFormat::Human => {
            if atty::is(Stream::Stdout) {
                print_table(projects);
            } else {
                for project in projects {
                    let id = project.id.clone().unwrap_or_default();
                    println!("{}|{}", id, project.name);
                }
            }
        }
    }
}
