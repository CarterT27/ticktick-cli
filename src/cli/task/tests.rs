use super::dates::{date_window_for, parse_task_date, task_due_date};
use super::filters::{
    normalize_list_name, parse_priority_shorthand, parse_task_status_value, parse_when_token,
};
use super::projects::{
    extract_inbox_tasks_from_value, normalize_project_id, task_project_id_or_fallback,
};
use super::*;
use chrono::{DateTime, NaiveDate};
use clap::Parser;
use iana_time_zone::get_timezone;
use serde_json::Value;

#[derive(Debug, Parser)]
struct TaskUpdateArgsCli {
    #[command(flatten)]
    args: TaskUpdateArgs,
}

fn make_task(
    due_date: Option<&str>,
    start_date: Option<&str>,
    tags: Option<Vec<&str>>,
    priority: Option<i32>,
) -> Task {
    Task {
        title: "sample".to_string(),
        due_date: due_date.map(ToString::to_string),
        start_date: start_date.map(ToString::to_string),
        tags: tags.map(|values| values.into_iter().map(ToString::to_string).collect()),
        priority,
        ..Default::default()
    }
}

#[test]
fn parses_priority_shorthand_case_insensitive() {
    assert_eq!(parse_priority_shorthand("!high"), Some(5));
    assert_eq!(parse_priority_shorthand("!High"), Some(5));
    assert_eq!(parse_priority_shorthand("!medium"), Some(3));
    assert_eq!(parse_priority_shorthand("!Low"), Some(1));
    assert_eq!(parse_priority_shorthand("!none"), Some(0));
    assert_eq!(parse_priority_shorthand("!urgent"), None);
}

#[test]
fn parses_priority_values_from_aliases_and_numbers() {
    assert_eq!(parse_priority_value("high"), Ok(5));
    assert_eq!(parse_priority_value("Medium"), Ok(3));
    assert_eq!(parse_priority_value("0"), Ok(0));
    assert_eq!(parse_priority_value("4"), Ok(4));
}

#[test]
fn rejects_invalid_priority_values_with_actionable_message() {
    let err = parse_priority_value("urgent").unwrap_err();
    assert!(err.contains("Invalid priority"));
    assert!(err.contains("none, low, medium, high"));
}

#[test]
fn parses_task_status_values_from_aliases() {
    assert_eq!(parse_task_status_value("done"), Ok(TaskStatus::Completed));
    assert_eq!(
        parse_task_status_value("Completed"),
        Ok(TaskStatus::Completed)
    );
    assert_eq!(parse_task_status_value("todo"), Ok(TaskStatus::Normal));
    assert_eq!(parse_task_status_value("OPEN"), Ok(TaskStatus::Normal));
}

#[test]
fn rejects_invalid_task_status_values() {
    let err = parse_task_status_value("blocked").unwrap_err();
    assert!(err.contains("Unsupported status"));
    assert!(err.contains("done, completed, todo, open"));
}

#[test]
fn parses_when_tokens() {
    assert_eq!(parse_when_token("today"), Some(TaskWhenFilter::Today));
    assert_eq!(parse_when_token("tomorrow"), Some(TaskWhenFilter::Tomorrow));
    assert_eq!(parse_when_token("week"), Some(TaskWhenFilter::ThisWeek));
    assert_eq!(
        parse_when_token("this-week"),
        Some(TaskWhenFilter::ThisWeek)
    );
    assert_eq!(parse_when_token("other"), None);
}

#[test]
fn parses_shorthand_markers_and_terms() {
    let parsed = parse_shorthand("finish report !High ~Personal #work #ops today");
    assert_eq!(parsed.priority, Some(5));
    assert_eq!(parsed.list.as_deref(), Some("Personal"));
    assert_eq!(parsed.when, Some(TaskWhenFilter::Today));
    assert_eq!(parsed.tags, vec!["work".to_string(), "ops".to_string()]);
    assert_eq!(
        parsed.terms,
        vec!["finish".to_string(), "report".to_string()]
    );
}

#[test]
fn parses_shorthand_this_week_phrase() {
    let parsed = parse_shorthand("plan this week");
    assert_eq!(parsed.when, Some(TaskWhenFilter::ThisWeek));
    assert_eq!(parsed.terms, vec!["plan".to_string()]);
}

#[test]
fn add_shorthand_keeps_when_terms_for_title() {
    let parsed = parse_task_add_shorthand("plan today");
    assert_eq!(parsed.when, None);
    assert_eq!(parsed.terms, vec!["plan".to_string(), "today".to_string()]);
}

#[test]
fn task_update_args_parse_extended_fields_and_clear_flags() {
    let parsed = TaskUpdateArgsCli::try_parse_from([
        "tt",
        "task-123",
        "--all-day",
        "false",
        "--status",
        "done",
        "--tags",
        "work",
        "--tags",
        "ops",
        "--clear-reminders",
    ])
    .unwrap()
    .args;

    assert_eq!(parsed.task_id, "task-123");
    assert_eq!(parsed.all_day, Some(false));
    assert_eq!(parsed.status, Some(TaskStatus::Completed));
    assert_eq!(parsed.tags, vec!["work".to_string(), "ops".to_string()]);
    assert!(parsed.clear_reminders);
}

#[test]
fn task_update_args_reject_conflicting_clear_and_set_flags() {
    let err = TaskUpdateArgsCli::try_parse_from([
        "tt",
        "task-123",
        "--due-date",
        "2026-03-26",
        "--clear-due-date",
    ])
    .unwrap_err();

    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn build_task_update_payload_includes_explicit_clears() {
    let task = Task {
        title: "sample".to_string(),
        start_date: Some("2026-03-01T00:00:00.000+0000".to_string()),
        due_date: Some("2026-03-02T00:00:00.000+0000".to_string()),
        time_zone: Some("America/Chicago".to_string()),
        tags: Some(vec!["work".to_string()]),
        reminders: Some(vec!["TRIGGER:P0DT9H0M0S".to_string()]),
        repeat_flag: Some("RRULE:FREQ=DAILY".to_string()),
        sort_order: Some(42),
        ..Default::default()
    };

    let payload = build_task_update_payload(
        &task,
        TaskUpdateClearFlags {
            start_date: true,
            due_date: true,
            time_zone: true,
            tags: true,
            reminders: true,
            repeat_flag: true,
            sort_order: true,
        },
    )
    .unwrap();

    assert_eq!(payload["startDate"], Value::Null);
    assert_eq!(payload["dueDate"], Value::Null);
    assert_eq!(payload["timeZone"], Value::Null);
    assert_eq!(payload["tags"], serde_json::json!([]));
    assert_eq!(payload["reminders"], serde_json::json!([]));
    assert_eq!(payload["repeatFlag"], Value::Null);
    assert_eq!(payload["sortOrder"], Value::Null);
}

#[test]
fn applies_system_time_zone_default_when_task_has_dates() {
    let mut task = Task {
        title: "sample".to_string(),
        due_date: Some("2026-03-02T00:00:00.000+0000".to_string()),
        ..Default::default()
    };

    apply_system_time_zone_default(&mut task).unwrap();

    assert_eq!(task.time_zone, Some(get_timezone().unwrap()));
}

#[test]
fn skips_system_time_zone_default_without_dates() {
    let mut task = Task {
        title: "sample".to_string(),
        ..Default::default()
    };

    apply_system_time_zone_default(&mut task).unwrap();

    assert_eq!(task.time_zone, None);
}

#[test]
fn preserves_existing_time_zone_when_present() {
    let mut task = Task {
        title: "sample".to_string(),
        due_date: Some("2026-03-02T00:00:00.000+0000".to_string()),
        time_zone: Some("Europe/Berlin".to_string()),
        ..Default::default()
    };

    apply_system_time_zone_default(&mut task).unwrap();

    assert_eq!(task.time_zone.as_deref(), Some("Europe/Berlin"));
}

#[test]
fn format_task_mutation_outputs_match_selected_mode() {
    let task = Task {
        id: Some("task-1".to_string()),
        title: "Inbox zero".to_string(),
        ..Default::default()
    };

    let created = format_task_create_output(&task, OutputFormat::Human).unwrap();
    assert!(created.contains("Task created: Inbox zero"));
    assert!(created.contains("ID: task-1"));

    let updated = format_task_update_output(&task, OutputFormat::Json).unwrap();
    assert!(updated.contains("\"title\": \"Inbox zero\""));

    let action =
        format_task_action_output("task-1", "project-1", "completed", OutputFormat::Json).unwrap();
    assert!(action.contains("\"status\": \"completed\""));
    assert!(action.contains("\"taskId\": \"task-1\""));
    assert!(action.contains("\"projectId\": \"project-1\""));
}

#[test]
fn extracts_due_date_today_and_cleans_title() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("finish report today", today);
    assert_eq!(title, "finish report");
    assert_eq!(date, Some(today));
}

#[test]
fn extracts_due_date_next_week_phrase() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("plan roadmap next week", today);
    assert_eq!(title, "plan roadmap");
    assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 2, 23).unwrap()));
}

#[test]
fn extracts_due_date_weekday() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
    let (title, date) = extract_due_date_from_input("ship draft friday", today);
    assert_eq!(title, "ship draft");
    assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 2, 20).unwrap()));
}

#[test]
fn extracts_due_date_numeric_month_day() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("pay rent 6/01", today);
    assert_eq!(title, "pay rent");
    assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()));
}

#[test]
fn extracts_due_date_text_month_day_year() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("renew passport feb 1 2027", today);
    assert_eq!(title, "renew passport");
    assert_eq!(date, Some(NaiveDate::from_ymd_opt(2027, 2, 1).unwrap()));
}

#[test]
fn keeps_hashtag_dates_as_tags() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("sync with team #friday", today);
    assert_eq!(title, "sync with team #friday");
    assert_eq!(date, None);
}

#[test]
fn extracts_due_date_text_month_year_short_name() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("plan launch jan 2029", today);
    assert_eq!(title, "plan launch");
    assert_eq!(date, Some(NaiveDate::from_ymd_opt(2029, 1, 1).unwrap()));
}

#[test]
fn extracts_due_date_text_month_year_full_name() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("plan launch january 2029", today);
    assert_eq!(title, "plan launch");
    assert_eq!(date, Some(NaiveDate::from_ymd_opt(2029, 1, 1).unwrap()));
}

#[test]
fn extracts_due_date_text_month_day_year_capitalized() {
    let today = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let (title, date) = extract_due_date_from_input("book trip January 3 2028", today);
    assert_eq!(title, "book trip");
    assert_eq!(date, Some(NaiveDate::from_ymd_opt(2028, 1, 3).unwrap()));
}

#[test]
fn formats_inferred_due_date_for_ticktick_api() {
    let date = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let value = format_ticktick_due_date(date).unwrap();
    assert!(DateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f%z").is_ok());
    assert!(value.ends_with("+0000"));
}

#[test]
fn normalizes_short_date_input_for_api_submission() {
    let value = normalize_task_datetime_input("2026-03-26").unwrap();
    assert_eq!(
        parse_task_date(&value),
        Some(NaiveDate::from_ymd_opt(2026, 3, 26).unwrap())
    );
}

#[test]
fn normalizes_iso_datetime_input_for_api_submission() {
    let value = normalize_task_datetime_input("2026-03-26T12:30:00+00:00").unwrap();
    assert!(DateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f%z").is_ok());
    assert_eq!(
        parse_task_date(&value),
        Some(NaiveDate::from_ymd_opt(2026, 3, 26).unwrap())
    );
}

#[test]
fn rejects_invalid_datetime_input_with_actionable_message() {
    let err = normalize_task_datetime_input("march sometime").unwrap_err();
    assert!(err.contains("Invalid date"));
    assert!(err.contains("YYYY-MM-DD"));
}

#[test]
fn merges_tags_without_case_duplicates() {
    let mut tags = vec!["work".to_string()];
    merge_tags(&mut tags, vec!["Work".to_string(), "ops".to_string()]);
    assert_eq!(tags, vec!["work".to_string(), "ops".to_string()]);
}

#[test]
fn matches_tags_case_insensitively() {
    let task = make_task(None, None, Some(vec!["Work", "ops"]), None);
    assert!(task_has_all_tags(
        &task,
        &["work".to_string(), "OPS".to_string()]
    ));
    assert!(!task_has_all_tags(&task, &["missing".to_string()]));
}

#[test]
fn normalizes_list_names_without_emoji() {
    assert_eq!(normalize_list_name("🚀Personal"), "personal");
    assert_eq!(normalize_list_name("👨🏻‍💻 Projects"), "projects");
    assert_eq!(normalize_list_name("Personal Team"), "personal team");
}

#[test]
fn detects_inbox_list_name_variants() {
    assert!(is_inbox_list_name("inbox"));
    assert!(is_inbox_list_name("Inbox"));
    assert!(is_inbox_list_name("  Inbox  "));
    assert!(is_inbox_list_name("📥 Inbox"));
    assert!(!is_inbox_list_name("work"));
}

#[test]
fn extracts_implicit_inbox_list_from_single_term() {
    let mut terms = vec!["inbox".to_string()];
    assert_eq!(
        extract_implicit_list_from_terms(&mut terms),
        Some("inbox".to_string())
    );
    assert!(terms.is_empty());

    let mut terms = vec!["inbox".to_string(), "urgent".to_string()];
    assert_eq!(extract_implicit_list_from_terms(&mut terms), None);
    assert_eq!(terms, vec!["inbox".to_string(), "urgent".to_string()]);
}

#[test]
fn extracts_inbox_tasks_from_multiple_payload_shapes() {
    let direct = serde_json::json!({
        "tasks": [
            {"id": "a", "title": "one", "projectId": "p"}
        ]
    });
    let wrapped = serde_json::json!({
        "data": {
            "tasks": [
                {"id": "b", "title": "two", "projectId": "p"}
            ]
        }
    });
    let array = serde_json::json!([
        {"id": "c", "title": "three", "projectId": "p"}
    ]);
    let sync = serde_json::json!({
        "syncTaskBean": {
            "update": [
                {"id": "d", "title": "four", "projectId": "p"}
            ]
        }
    });

    assert_eq!(extract_inbox_tasks_from_value(&direct).unwrap().len(), 1);
    assert_eq!(extract_inbox_tasks_from_value(&wrapped).unwrap().len(), 1);
    assert_eq!(extract_inbox_tasks_from_value(&array).unwrap().len(), 1);
    assert_eq!(extract_inbox_tasks_from_value(&sync).unwrap().len(), 1);
}

#[test]
fn normalizes_project_ids() {
    assert_eq!(normalize_project_id(None), None);
    assert_eq!(normalize_project_id(Some("".to_string())), None);
    assert_eq!(normalize_project_id(Some("   ".to_string())), None);
    assert_eq!(
        normalize_project_id(Some("  abc123  ".to_string())),
        Some("abc123".to_string())
    );
}

#[test]
fn task_project_id_prefers_task_and_falls_back_to_container() {
    let mut task = Task {
        title: "sample".to_string(),
        ..Default::default()
    };
    task.project_id = Some("real-project".to_string());
    assert_eq!(
        task_project_id_or_fallback(&task, ""),
        Some("real-project".to_string())
    );

    task.project_id = None;
    assert_eq!(
        task_project_id_or_fallback(&task, "container-project"),
        Some("container-project".to_string())
    );

    assert_eq!(task_project_id_or_fallback(&task, "  "), None);
}

#[test]
fn parses_task_date_from_iso_and_prefix() {
    assert_eq!(
        parse_task_date("2026-03-01T00:00:00.000+0000"),
        Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap())
    );
    assert_eq!(
        parse_task_date("2026-03-01T00:00:00"),
        Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap())
    );
    assert_eq!(
        parse_task_date("2026-03-01"),
        Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap())
    );
}

#[test]
fn parses_task_date_from_epoch_values() {
    assert_eq!(
        parse_task_date("1704067200000"),
        Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
    );
    assert_eq!(
        parse_task_date("1704067200"),
        Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
    );
}

#[test]
fn computes_date_windows() {
    let base = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    assert_eq!(date_window_for(TaskWhenFilter::Today, base), (base, base));
    assert_eq!(
        date_window_for(TaskWhenFilter::Tomorrow, base),
        (
            NaiveDate::from_ymd_opt(2026, 2, 21).unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 21).unwrap()
        )
    );
    assert_eq!(
        date_window_for(TaskWhenFilter::ThisWeek, base),
        (
            NaiveDate::from_ymd_opt(2026, 2, 16).unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 22).unwrap()
        )
    );
}

#[test]
fn filters_tasks_for_when() {
    let base = NaiveDate::from_ymd_opt(2026, 2, 20).unwrap();
    let today = make_task(Some("2026-02-20"), None, None, None);
    let tomorrow = make_task(Some("2026-02-21"), None, None, None);
    let this_week = make_task(Some("2026-02-22"), None, None, None);
    let next_week = make_task(Some("2026-02-23"), None, None, None);
    let no_date = make_task(None, None, None, None);

    assert!(task_matches_when_filter(
        &today,
        TaskWhenFilter::Today,
        base
    ));
    assert!(!task_matches_when_filter(
        &tomorrow,
        TaskWhenFilter::Today,
        base
    ));
    assert!(task_matches_when_filter(
        &tomorrow,
        TaskWhenFilter::Tomorrow,
        base
    ));
    assert!(task_matches_when_filter(
        &this_week,
        TaskWhenFilter::ThisWeek,
        base
    ));
    assert!(!task_matches_when_filter(
        &next_week,
        TaskWhenFilter::ThisWeek,
        base
    ));
    assert!(!task_matches_when_filter(
        &no_date,
        TaskWhenFilter::Today,
        base
    ));
}

#[test]
fn uses_due_date_then_start_date() {
    let task = make_task(None, Some("2026-03-02"), None, None);
    assert_eq!(
        task_due_date(&task),
        Some(NaiveDate::from_ymd_opt(2026, 3, 2).unwrap())
    );
}

#[test]
fn parses_query_with_unknown_bang_as_term() {
    let parsed = parse_shorthand("review !urgent");
    assert_eq!(parsed.priority, None);
    assert_eq!(
        parsed.terms,
        vec!["review".to_string(), "!urgent".to_string()]
    );
}

#[test]
fn parse_task_date_rejects_invalid_values() {
    assert_eq!(parse_task_date(""), None);
    assert_eq!(parse_task_date("not-a-date"), None);
}

#[test]
fn treats_non_terminal_task_statuses_as_open() {
    let active: Task = serde_json::from_value(serde_json::json!({
        "title": "Investigate parser bug",
        "status": 1
    }))
    .unwrap();
    let completed = Task {
        title: "Ship fix".to_string(),
        status: Some(TaskStatus::Completed),
        ..Default::default()
    };

    assert!(!task_is_completed(&active));
    assert!(task_is_completed(&completed));
}

#[test]
fn make_task_helper_sets_priority() {
    let task = make_task(Some("2026-03-01"), None, None, Some(3));
    assert_eq!(task.priority, Some(3));
}

#[test]
fn syncs_desc_into_content_when_content_missing() {
    let mut task = Task {
        title: "sample".to_string(),
        desc: Some("details".to_string()),
        ..Default::default()
    };

    sync_task_note_fields(&mut task);

    assert_eq!(task.content.as_deref(), Some("details"));
    assert_eq!(task.desc.as_deref(), Some("details"));
}

#[test]
fn syncs_content_into_desc_when_desc_missing() {
    let mut task = Task {
        title: "sample".to_string(),
        content: Some("details".to_string()),
        ..Default::default()
    };

    sync_task_note_fields(&mut task);

    assert_eq!(task.content.as_deref(), Some("details"));
    assert_eq!(task.desc.as_deref(), Some("details"));
}

#[test]
fn preserves_distinct_note_fields_when_both_exist() {
    let mut task = Task {
        title: "sample".to_string(),
        content: Some("content".to_string()),
        desc: Some("desc".to_string()),
        ..Default::default()
    };

    sync_task_note_fields(&mut task);

    assert_eq!(task.content.as_deref(), Some("content"));
    assert_eq!(task.desc.as_deref(), Some("desc"));
}

#[test]
fn resolve_task_note_fields_mirrors_desc_when_content_not_provided() {
    let (content, desc) = resolve_task_note_fields(None, Some("details".to_string()));

    assert_eq!(content.as_deref(), Some("details"));
    assert_eq!(desc.as_deref(), Some("details"));
}

#[test]
fn resolve_task_note_fields_mirrors_content_when_desc_not_provided() {
    let (content, desc) = resolve_task_note_fields(Some("details".to_string()), None);

    assert_eq!(content.as_deref(), Some("details"));
    assert_eq!(desc.as_deref(), Some("details"));
}

#[test]
fn resolve_task_note_fields_preserves_distinct_explicit_values() {
    let (content, desc) =
        resolve_task_note_fields(Some("content".to_string()), Some("desc".to_string()));

    assert_eq!(content.as_deref(), Some("content"));
    assert_eq!(desc.as_deref(), Some("desc"));
}
