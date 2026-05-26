use crate::models::Task;
use chrono::{
    DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday,
};

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskWhenFilter {
    #[value(alias = "late")]
    Overdue,
    Today,
    Tomorrow,
    #[value(alias = "thisweek", alias = "this-week", alias = "week")]
    ThisWeek,
}

fn normalize_date_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '/' && ch != '-')
        .to_ascii_lowercase()
}

fn infer_year_for_month_day(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if this_year >= today {
        Some(this_year)
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, month, day)
    }
}

fn parse_year_token(token: &str) -> Option<i32> {
    let year = token.parse::<i32>().ok()?;
    match token.len() {
        2 => Some(2000 + year),
        4 => Some(year),
        _ => None,
    }
}

fn parse_day_token(token: &str) -> Option<u32> {
    let day_text = token
        .strip_suffix("st")
        .or_else(|| token.strip_suffix("nd"))
        .or_else(|| token.strip_suffix("rd"))
        .or_else(|| token.strip_suffix("th"))
        .unwrap_or(token);

    let day = day_text.parse::<u32>().ok()?;
    if (1..=31).contains(&day) {
        Some(day)
    } else {
        None
    }
}

fn parse_month_token(token: &str) -> Option<u32> {
    match token {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_weekday_token(token: &str) -> Option<Weekday> {
    match token {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tues" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thurs" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn next_or_same_weekday(today: NaiveDate, target: Weekday) -> NaiveDate {
    let today_idx = today.weekday().num_days_from_monday() as i64;
    let target_idx = target.num_days_from_monday() as i64;
    let offset = (target_idx - today_idx + 7) % 7;
    today + Duration::days(offset)
}

fn start_of_next_week(today: NaiveDate) -> NaiveDate {
    let start_of_this_week = today - Duration::days(today.weekday().num_days_from_monday().into());
    start_of_this_week + Duration::days(7)
}

fn parse_numeric_date_token(token: &str, today: NaiveDate) -> Option<NaiveDate> {
    if let Ok(date) = NaiveDate::parse_from_str(token, "%Y-%m-%d") {
        return Some(date);
    }

    let separator = if token.contains('/') {
        Some('/')
    } else if token.matches('-').count() == 2 {
        Some('-')
    } else {
        None
    }?;

    let parts: Vec<&str> = token.split(separator).collect();
    if parts.len() == 2 {
        let month = parts[0].parse::<u32>().ok()?;
        let day = parts[1].parse::<u32>().ok()?;
        return infer_year_for_month_day(month, day, today);
    }

    if parts.len() == 3 {
        let month = parts[0].parse::<u32>().ok()?;
        let day = parts[1].parse::<u32>().ok()?;
        let year = parse_year_token(parts[2])?;
        return NaiveDate::from_ymd_opt(year, month, day);
    }

    None
}

fn parse_month_day_sequence(
    tokens: &[&str],
    index: usize,
    today: NaiveDate,
) -> Option<(usize, NaiveDate)> {
    let month = parse_month_token(&normalize_date_token(tokens.get(index)?))?;
    let second = normalize_date_token(tokens.get(index + 1)?);

    if let Some(year) = parse_year_token(&second) {
        let date = NaiveDate::from_ymd_opt(year, month, 1)?;
        return Some((2, date));
    }

    let day = parse_day_token(&second)?;

    if let Some(year_token) = tokens.get(index + 2) {
        let normalized_year = normalize_date_token(year_token);
        if let Some(year) = parse_year_token(&normalized_year) {
            let date = NaiveDate::from_ymd_opt(year, month, day)?;
            return Some((3, date));
        }
    }

    let date = infer_year_for_month_day(month, day, today)?;
    Some((2, date))
}

pub(super) fn extract_due_date_from_input(
    raw: &str,
    today: NaiveDate,
) -> (String, Option<NaiveDate>) {
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    if tokens.is_empty() {
        return (String::new(), None);
    }

    for (index, token) in tokens.iter().enumerate() {
        if token.starts_with('#') || token.starts_with('~') || token.starts_with('!') {
            continue;
        }

        let normalized = normalize_date_token(token);
        if normalized.is_empty() {
            continue;
        }

        if normalized == "next"
            && index + 1 < tokens.len()
            && normalize_date_token(tokens[index + 1]) == "week"
        {
            let date = start_of_next_week(today);
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| {
                    if i == index || i == index + 1 {
                        None
                    } else {
                        Some(*value)
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }

        if let Some((consumed, date)) = parse_month_day_sequence(&tokens, index, today) {
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| {
                    if i >= index && i < index + consumed {
                        None
                    } else {
                        Some(*value)
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }

        if let Some(date) = parse_numeric_date_token(&normalized, today) {
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| if i == index { None } else { Some(*value) })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }

        let relative_date = match normalized.as_str() {
            "today" => Some(today),
            "tomorrow" => Some(today + Duration::days(1)),
            _ => {
                parse_weekday_token(&normalized).map(|weekday| next_or_same_weekday(today, weekday))
            }
        };

        if let Some(date) = relative_date {
            let title = tokens
                .iter()
                .enumerate()
                .filter_map(|(i, value)| if i == index { None } else { Some(*value) })
                .collect::<Vec<_>>()
                .join(" ");
            return (title, Some(date));
        }
    }

    (raw.trim().to_string(), None)
}

pub(super) fn format_ticktick_due_date(date: NaiveDate) -> Option<String> {
    let local_midnight = date.and_hms_opt(0, 0, 0)?;
    let local_dt = Local
        .from_local_datetime(&local_midnight)
        .earliest()
        .or_else(|| Local.from_local_datetime(&local_midnight).latest())?;
    let utc_dt = local_dt.with_timezone(&Utc);
    Some(utc_dt.format("%Y-%m-%dT%H:%M:%S%.3f+0000").to_string())
}

fn format_ticktick_datetime<Tz: TimeZone>(dt: DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    dt.with_timezone(&Utc)
        .format("%Y-%m-%dT%H:%M:%S%.3f+0000")
        .to_string()
}

fn parse_local_datetime(value: &str, format: &str) -> Option<String> {
    let naive = NaiveDateTime::parse_from_str(value, format).ok()?;
    let local = Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())?;
    Some(format_ticktick_datetime(local))
}

pub(super) fn normalize_task_datetime_input(value: &str) -> std::result::Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Date value cannot be empty.".to_string());
    }

    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return format_ticktick_due_date(date).ok_or_else(|| {
            format!(
                "Failed to format date '{}'. Use YYYY-MM-DD or ISO 8601.",
                value
            )
        });
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(format_ticktick_datetime(dt));
    }

    for format in ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%dT%H:%M:%S%z"] {
        if let Ok(dt) = DateTime::parse_from_str(trimmed, format) {
            return Ok(format_ticktick_datetime(dt));
        }
    }

    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Some(dt) = parse_local_datetime(trimmed, format) {
            return Ok(dt);
        }
    }

    Err(format!(
        "Invalid date '{}'. Use YYYY-MM-DD or ISO 8601 like 2026-03-26T00:00:00+0000.",
        value
    ))
}

pub(super) fn parse_task_date(value: &str) -> Option<NaiveDate> {
    if let Ok(epoch) = value.parse::<i64>() {
        let dt = if value.len() > 10 {
            DateTime::<Utc>::from_timestamp_millis(epoch)?
        } else {
            DateTime::<Utc>::from_timestamp(epoch, 0)?
        };
        return Some(dt.date_naive());
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Some(dt.date_naive());
    }

    if let Ok(dt) = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(dt.date_naive());
    }

    if let Ok(dt) = DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(dt.date_naive());
    }

    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date);
    }

    let prefix = value.get(0..10)?;
    NaiveDate::parse_from_str(prefix, "%Y-%m-%d").ok()
}

pub(super) fn task_due_date(task: &Task) -> Option<NaiveDate> {
    task.due_date
        .as_deref()
        .or(task.start_date.as_deref())
        .and_then(parse_task_date)
}

pub(super) fn date_window_for(when: TaskWhenFilter, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match when {
        TaskWhenFilter::Overdue => (NaiveDate::MIN, today - Duration::days(1)),
        TaskWhenFilter::Today => (today, today),
        TaskWhenFilter::Tomorrow => {
            let day = today + Duration::days(1);
            (day, day)
        }
        TaskWhenFilter::ThisWeek => {
            let start = today - Duration::days(today.weekday().num_days_from_monday().into());
            let end = start + Duration::days(6);
            (start, end)
        }
    }
}

pub(super) fn task_matches_when_filter(
    task: &Task,
    when: TaskWhenFilter,
    today: NaiveDate,
) -> bool {
    let Some(task_date) = task_due_date(task) else {
        return false;
    };

    if matches!(when, TaskWhenFilter::Today) {
        return task_date <= today;
    }

    let (start, end) = date_window_for(when, today);
    task_date >= start && task_date <= end
}
