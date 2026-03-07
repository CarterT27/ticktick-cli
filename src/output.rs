use crate::models::{Project, Task};
use atty::Stream;
use serde::Serialize;
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

fn render_table<T: Tabular>(items: &[T]) -> String {
    if items.is_empty() {
        return "No items found.\n".to_string();
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

    let mut output = String::new();
    output.push_str(&format!("|{}|\n", header_row));
    output.push_str(&format!("|{}|\n", separator));

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
        output.push_str(&format!("|{}|\n", row_str));
    }

    output
}

fn render_json<T: Serialize>(items: &[T]) -> String {
    let mut output = serde_json::to_string_pretty(items).unwrap_or_else(|_| "[]".to_string());
    output.push('\n');
    output
}

fn render_task_lines(tasks: &[Task]) -> String {
    let mut output = tasks
        .iter()
        .map(|task| {
            let id = task.id.clone().unwrap_or_default();
            format!("{}|{}", id, task.title)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

fn render_project_lines(projects: &[Project]) -> String {
    let mut output = projects
        .iter()
        .map(|project| {
            let id = project.id.clone().unwrap_or_default();
            format!("{}|{}", id, project.name)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

fn render_tasks(tasks: &[Task], format: OutputFormat, is_tty: bool) -> String {
    match format {
        OutputFormat::Json => render_json(tasks),
        OutputFormat::Human => {
            if is_tty {
                render_table(tasks)
            } else {
                render_task_lines(tasks)
            }
        }
    }
}

fn render_projects(projects: &[Project], format: OutputFormat, is_tty: bool) -> String {
    match format {
        OutputFormat::Json => render_json(projects),
        OutputFormat::Human => {
            if is_tty {
                render_table(projects)
            } else {
                render_project_lines(projects)
            }
        }
    }
}

pub fn print_tasks(tasks: &[Task], format: OutputFormat) {
    let _ = io::Write::write_all(
        &mut io::stdout(),
        render_tasks(tasks, format, atty::is(Stream::Stdout)).as_bytes(),
    );
}

pub fn print_projects(projects: &[Project], format: OutputFormat) {
    let _ = io::Write::write_all(
        &mut io::stdout(),
        render_projects(projects, format, atty::is(Stream::Stdout)).as_bytes(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_rows_format_priority_due_date_and_missing_id() {
        let task = Task {
            title: "Ship release".to_string(),
            priority: Some(5),
            due_date: Some("2026-03-08T09:00:00Z".to_string()),
            ..Default::default()
        };

        assert_eq!(
            task.rows(),
            vec![
                "".to_string(),
                "Ship release".to_string(),
                "High".to_string(),
                "2026-03-08".to_string(),
            ]
        );
    }

    #[test]
    fn project_rows_truncate_long_ids() {
        let project = Project {
            id: Some("1234567890abcdef".to_string()),
            name: "Inbox".to_string(),
            color: Some("#ff0000".to_string()),
            view_mode: Some("list".to_string()),
            ..Default::default()
        };

        assert_eq!(
            project.rows(),
            vec![
                "12345678...".to_string(),
                "Inbox".to_string(),
                "#ff0000".to_string(),
                "list".to_string(),
            ]
        );
    }

    #[test]
    fn render_table_handles_empty_lists() {
        let tasks: Vec<Task> = Vec::new();
        assert_eq!(render_table(&tasks), "No items found.\n");
    }

    #[test]
    fn render_tasks_uses_pipe_output_for_non_tty_human_mode() {
        let tasks = vec![Task {
            id: Some("task-1".to_string()),
            title: "Write tests".to_string(),
            ..Default::default()
        }];

        assert_eq!(
            render_tasks(&tasks, OutputFormat::Human, false),
            "task-1|Write tests\n"
        );
    }

    #[test]
    fn render_projects_supports_json_and_tty_table_output() {
        let projects = vec![Project {
            id: Some("123456789".to_string()),
            name: "Inbox".to_string(),
            color: Some("#00ff00".to_string()),
            view_mode: Some("kanban".to_string()),
            ..Default::default()
        }];

        let json = render_projects(&projects, OutputFormat::Json, false);
        assert!(json.contains("\"name\": \"Inbox\""));

        let table = render_projects(&projects, OutputFormat::Human, true);
        assert!(table.contains("| ID"));
        assert!(table.contains("12345678..."));
        assert!(table.contains("kanban"));
    }
}
