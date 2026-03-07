use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

fn deserialize_opt_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Number(n)) => Ok(Some(n.to_string())),
        Some(serde_json::Value::Bool(b)) => Ok(Some(b.to_string())),
        Some(other) => serde_json::to_string(&other)
            .map(Some)
            .map_err(de::Error::custom),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub completed_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_all_day: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_all_day: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub completed_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub due_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<ChecklistItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminders: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_flag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Normal,
    Completed,
}

impl Serialize for TaskStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let value = match self {
            TaskStatus::Normal => 0,
            TaskStatus::Completed => 2,
        };
        serializer.serialize_i32(value)
    }
}

impl<'de> Deserialize<'de> for TaskStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum TaskStatusRepr {
            Int(i32),
            Str(String),
        }

        let repr = TaskStatusRepr::deserialize(deserializer)?;
        let value = match repr {
            TaskStatusRepr::Int(v) => v,
            TaskStatusRepr::Str(s) => s
                .parse::<i32>()
                .map_err(|_| de::Error::custom(format!("Unsupported task status: {}", s)))?,
        };

        match value {
            0 => Ok(TaskStatus::Normal),
            2 => Ok(TaskStatus::Completed),
            _ => Err(de::Error::custom(format!(
                "Unsupported task status value: {}",
                value
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub project_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectData {
    pub project: Project,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<Task>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<Column>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize)]
    struct OptionalStringWrapper {
        #[serde(default, deserialize_with = "deserialize_opt_string")]
        value: Option<String>,
    }

    #[test]
    fn deserialize_opt_string_accepts_scalars_and_nested_values() {
        let number: OptionalStringWrapper = serde_json::from_value(json!({ "value": 42 })).unwrap();
        assert_eq!(number.value.as_deref(), Some("42"));

        let boolean: OptionalStringWrapper =
            serde_json::from_value(json!({ "value": true })).unwrap();
        assert_eq!(boolean.value.as_deref(), Some("true"));

        let object: OptionalStringWrapper =
            serde_json::from_value(json!({ "value": { "nested": "value" } })).unwrap();
        assert_eq!(object.value.as_deref(), Some("{\"nested\":\"value\"}"));

        let null_value: OptionalStringWrapper =
            serde_json::from_value(json!({ "value": null })).unwrap();
        assert_eq!(null_value.value, None);
    }

    #[test]
    fn task_status_serializes_and_deserializes_supported_values() {
        assert_eq!(serde_json::to_string(&TaskStatus::Normal).unwrap(), "0");
        assert_eq!(serde_json::to_string(&TaskStatus::Completed).unwrap(), "2");

        assert_eq!(
            serde_json::from_value::<TaskStatus>(json!(0)).unwrap(),
            TaskStatus::Normal
        );
        assert_eq!(
            serde_json::from_value::<TaskStatus>(json!("2")).unwrap(),
            TaskStatus::Completed
        );
    }

    #[test]
    fn task_status_rejects_unsupported_values() {
        let err = serde_json::from_value::<TaskStatus>(json!(1))
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unsupported task status value: 1"));
    }

    #[test]
    fn task_deserialization_normalizes_non_string_date_fields() {
        let task: Task = serde_json::from_value(json!({
            "title": "Review PR",
            "dueDate": 1710000000,
            "completedTime": false,
            "startDate": { "seconds": 30 },
            "status": "0"
        }))
        .unwrap();

        assert_eq!(task.title, "Review PR");
        assert_eq!(task.due_date.as_deref(), Some("1710000000"));
        assert_eq!(task.completed_time.as_deref(), Some("false"));
        assert_eq!(task.start_date.as_deref(), Some("{\"seconds\":30}"));
        assert_eq!(task.status, Some(TaskStatus::Normal));
    }
}
