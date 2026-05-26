use super::dates::TaskWhenFilter;
use crate::models::{Task, TaskStatus};

#[derive(Default)]
pub(super) struct ShorthandFilters {
    pub(super) priority: Option<i32>,
    pub(super) list: Option<String>,
    pub(super) tags: Vec<String>,
    pub(super) when: Option<TaskWhenFilter>,
    pub(super) terms: Vec<String>,
}

pub(super) fn parse_priority_shorthand(token: &str) -> Option<i32> {
    let value = token.strip_prefix('!')?.to_ascii_lowercase();
    match value.as_str() {
        "high" => Some(5),
        "medium" => Some(3),
        "low" => Some(1),
        "none" | "normal" => Some(0),
        _ => None,
    }
}

pub(super) fn parse_priority_value(value: &str) -> std::result::Result<i32, String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "none" | "normal" => Ok(0),
        "low" => Ok(1),
        "medium" => Ok(3),
        "high" => Ok(5),
        _ => value.trim().parse::<i32>().map_err(|_| {
            format!(
                "Invalid priority '{}'. Use an integer or one of: none, low, medium, high.",
                value
            )
        }),
    }
}

pub(super) fn parse_task_status_value(value: &str) -> std::result::Result<TaskStatus, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "done" | "completed" => Ok(TaskStatus::Completed),
        "todo" | "open" => Ok(TaskStatus::Normal),
        _ => Err(format!(
            "Unsupported status '{}'. Use one of: done, completed, todo, open",
            value
        )),
    }
}

pub(super) fn parse_when_token(token: &str) -> Option<TaskWhenFilter> {
    match token.to_ascii_lowercase().as_str() {
        "overdue" | "late" => Some(TaskWhenFilter::Overdue),
        "today" => Some(TaskWhenFilter::Today),
        "tomorrow" => Some(TaskWhenFilter::Tomorrow),
        "week" | "thisweek" | "this-week" => Some(TaskWhenFilter::ThisWeek),
        _ => None,
    }
}

fn parse_shorthand_with_when(raw: &str, parse_when: bool) -> ShorthandFilters {
    let mut parsed = ShorthandFilters::default();
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let mut index = 0;

    while index < tokens.len() {
        let token = tokens[index];
        if let Some(priority) = parse_priority_shorthand(token) {
            parsed.priority = Some(priority);
            index += 1;
            continue;
        }

        if let Some(list) = token.strip_prefix('~') {
            if !list.is_empty() {
                parsed.list = Some(list.to_string());
                index += 1;
                continue;
            }
        }

        if let Some(tag) = token.strip_prefix('#') {
            if !tag.is_empty() {
                parsed.tags.push(tag.to_string());
                index += 1;
                continue;
            }
        }

        if parse_when {
            if token.eq_ignore_ascii_case("this")
                && index + 1 < tokens.len()
                && tokens[index + 1].eq_ignore_ascii_case("week")
            {
                parsed.when = Some(TaskWhenFilter::ThisWeek);
                index += 2;
                continue;
            }

            if let Some(when) = parse_when_token(token) {
                parsed.when = Some(when);
                index += 1;
                continue;
            }
        }

        parsed.terms.push(token.to_string());
        index += 1;
    }

    parsed
}

pub(super) fn parse_shorthand(raw: &str) -> ShorthandFilters {
    parse_shorthand_with_when(raw, true)
}

pub(super) fn parse_task_add_shorthand(raw: &str) -> ShorthandFilters {
    parse_shorthand_with_when(raw, false)
}

pub(super) fn merge_tags(existing: &mut Vec<String>, extras: Vec<String>) {
    for tag in extras {
        if !existing
            .iter()
            .any(|current| current.eq_ignore_ascii_case(&tag))
        {
            existing.push(tag);
        }
    }
}

pub(super) fn task_has_all_tags(task: &Task, required_tags: &[String]) -> bool {
    let Some(task_tags) = task.tags.as_ref() else {
        return false;
    };

    required_tags.iter().all(|required| {
        task_tags
            .iter()
            .any(|actual| actual.eq_ignore_ascii_case(required))
    })
}

pub(super) fn normalize_list_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn is_inbox_list_name(value: &str) -> bool {
    value.eq_ignore_ascii_case("inbox") || normalize_list_name(value) == "inbox"
}

pub(super) fn extract_implicit_list_from_terms(terms: &mut Vec<String>) -> Option<String> {
    if terms.len() == 1 && is_inbox_list_name(&terms[0]) {
        return Some(terms.remove(0));
    }

    None
}
