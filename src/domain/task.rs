use ratatui::style::{Color, Style};
use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::domain::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    Done,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::Done => "done",
        }
    }

    pub fn try_from_str(value: &str) -> Result<Self, AppError> {
        match value {
            "todo" => Ok(Status::Todo),
            "in_progress" => Ok(Status::InProgress),
            "done" => Ok(Status::Done),
            _ => Err(AppError::Validation(format!("invalid status: {value}"))),
        }
    }

    pub fn next(self) -> Status {
        match self {
            Status::Todo => Status::InProgress,
            Status::InProgress => Status::Done,
            Status::Done => Status::Todo,
        }
    }

    pub fn previous(self) -> Status {
        match self {
            Status::Todo => Status::Done,
            Status::InProgress => Status::Todo,
            Status::Done => Status::InProgress,
        }
    }

    pub fn cycle_next(self) -> Option<Status> {
        match self {
            Status::Todo => Some(Status::InProgress),
            Status::InProgress => Some(Status::Done),
            Status::Done => None,
        }
    }

    pub fn cycle_previous(self) -> Option<Status> {
        match self {
            Status::Todo => None,
            Status::InProgress => Some(Status::Todo),
            Status::Done => Some(Status::InProgress),
        }
    }

    pub fn all() -> [Status; 3] {
        [Status::Todo, Status::InProgress, Status::Done]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Status::Todo => "Todo",
            Status::InProgress => "In Progress",
            Status::Done => "Done",
        }
    }

    pub fn style(&self) -> Style {
        match self {
            Status::Todo => Style::default().fg(Color::Yellow),
            Status::InProgress => Style::default().fg(Color::Blue),
            Status::Done => Style::default().fg(Color::Green),
        }
    }

    pub fn row_style(&self) -> Style {
        match self {
            Status::Done => Style::default().fg(Color::DarkGray),
            _ => Style::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, schemars::JsonSchema)]
#[repr(i64)]
pub enum Priority {
    Low = 1,
    Medium = 2,
    High = 3,
}

impl Priority {
    pub fn next(self) -> Priority {
        match self {
            Priority::Low => Priority::Medium,
            Priority::Medium => Priority::High,
            Priority::High => Priority::Low,
        }
    }

    pub fn previous(self) -> Priority {
        match self {
            Priority::Low => Priority::High,
            Priority::Medium => Priority::Low,
            Priority::High => Priority::Medium,
        }
    }

    pub fn cycle_next(self) -> Option<Priority> {
        match self {
            Priority::Low => Some(Priority::Medium),
            Priority::Medium => Some(Priority::High),
            Priority::High => None,
        }
    }

    pub fn cycle_previous(self) -> Option<Priority> {
        match self {
            Priority::Low => None,
            Priority::Medium => Some(Priority::Low),
            Priority::High => Some(Priority::Medium),
        }
    }

    pub fn all() -> [Priority; 3] {
        [Priority::Low, Priority::Medium, Priority::High]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Priority::Low => "Low",
            Priority::Medium => "Medium",
            Priority::High => "High",
        }
    }

    pub fn style(&self) -> Style {
        match self {
            Priority::High => Style::default().fg(Color::Red),
            Priority::Medium => Style::default().fg(Color::Yellow),
            Priority::Low => Style::default().fg(Color::Green),
        }
    }
}

impl From<Priority> for i64 {
    fn from(priority: Priority) -> Self {
        priority as i64
    }
}

impl TryFrom<i64> for Priority {
    type Error = AppError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Priority::Low),
            2 => Ok(Priority::Medium),
            3 => Ok(Priority::High),
            _ => Err(AppError::Validation(format!("invalid priority: {value}"))),
        }
    }
}

impl Serialize for Priority {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i64((*self).into())
    }
}

impl<'de> Deserialize<'de> for Priority {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = i64::deserialize(deserializer)?;
        Priority::try_from(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: Option<i64>,
    pub project_id: i64,
    pub name: String,
    pub description: String,
    pub status: Status,
    pub priority: Priority,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskDependency {
    pub task_id: i64,
    pub depends_on_id: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskNote {
    pub id: Option<i64>,
    pub task_id: i64,
    pub content: String,
    pub created_at: String,
}

impl TaskNote {
    /// Creates a not-yet-persisted note, enforcing the content invariant.
    ///
    /// This is the single source of truth for note validation: every
    /// `Repository` implementation uses it so that empty or
    /// whitespace-only content is rejected uniformly with
    /// `AppError::Validation`. `id` and `created_at` are assigned by the
    /// repository on persistence.
    pub fn new(task_id: i64, content: &str) -> Result<TaskNote, AppError> {
        if content.trim().is_empty() {
            return Err(AppError::Validation(
                "note content cannot be empty".to_string(),
            ));
        }
        Ok(TaskNote {
            id: None,
            task_id,
            content: content.to_string(),
            created_at: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    #[test]
    fn status_serializes_to_snake_case_strings() {
        assert_eq!(serde_json::to_string(&Status::Todo).unwrap(), "\"todo\"");
        assert_eq!(
            serde_json::to_string(&Status::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(serde_json::to_string(&Status::Done).unwrap(), "\"done\"");
    }

    #[test]
    fn status_round_trips_all_variants() {
        for status in [Status::Todo, Status::InProgress, Status::Done] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: Status = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn priority_serializes_to_integers() {
        assert_eq!(serde_json::to_string(&Priority::Low).unwrap(), "1");
        assert_eq!(serde_json::to_string(&Priority::Medium).unwrap(), "2");
        assert_eq!(serde_json::to_string(&Priority::High).unwrap(), "3");
    }

    #[test]
    fn priority_round_trips_all_variants() {
        for priority in [Priority::Low, Priority::Medium, Priority::High] {
            let json = serde_json::to_string(&priority).unwrap();
            let parsed: Priority = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, priority);
        }
    }

    #[test]
    fn priority_try_from_rejects_invalid_values() {
        assert!(Priority::try_from(0).is_err());
        assert!(Priority::try_from(4).is_err());
        assert!(Priority::try_from(-1).is_err());
    }

    #[test]
    fn priority_from_yields_integers() {
        assert_eq!(i64::from(Priority::Low), 1);
        assert_eq!(i64::from(Priority::Medium), 2);
        assert_eq!(i64::from(Priority::High), 3);
    }

    #[test]
    fn task_fields_match_spec() {
        let task = Task {
            id: None,
            project_id: 1,
            name: "Task A".to_string(),
            description: "".to_string(),
            status: Status::Todo,
            priority: Priority::Medium,
            created_at: "now".to_string(),
        };
        assert!(task.id.is_none());
        assert_eq!(task.project_id, 1);
        assert_eq!(task.name, "Task A");
        assert_eq!(task.status, Status::Todo);
        assert_eq!(task.priority, Priority::Medium);
    }

    #[test]
    fn status_cycles_to_next_status() {
        assert_eq!(Status::Todo.next(), Status::InProgress);
        assert_eq!(Status::InProgress.next(), Status::Done);
        assert_eq!(Status::Done.next(), Status::Todo);
    }

    #[test]
    fn status_cycles_to_previous_status() {
        assert_eq!(Status::Todo.previous(), Status::Done);
        assert_eq!(Status::InProgress.previous(), Status::Todo);
        assert_eq!(Status::Done.previous(), Status::InProgress);
    }

    #[test]
    fn status_all_contains_all_variants() {
        assert_eq!(Status::all().len(), 3);
        assert!(Status::all().contains(&Status::Todo));
        assert!(Status::all().contains(&Status::InProgress));
        assert!(Status::all().contains(&Status::Done));
    }

    #[test]
    fn priority_cycles_to_next_priority() {
        assert_eq!(Priority::Low.next(), Priority::Medium);
        assert_eq!(Priority::Medium.next(), Priority::High);
        assert_eq!(Priority::High.next(), Priority::Low);
    }

    #[test]
    fn priority_cycles_to_previous_priority() {
        assert_eq!(Priority::Low.previous(), Priority::High);
        assert_eq!(Priority::Medium.previous(), Priority::Low);
        assert_eq!(Priority::High.previous(), Priority::Medium);
    }

    #[test]
    fn priority_all_contains_all_variants() {
        assert_eq!(Priority::all().len(), 3);
        assert!(Priority::all().contains(&Priority::Low));
        assert!(Priority::all().contains(&Priority::Medium));
        assert!(Priority::all().contains(&Priority::High));
    }

    #[test]
    fn task_dependency_fields_match_spec() {
        let dep = TaskDependency {
            task_id: 7,
            depends_on_id: 3,
        };
        assert_eq!(dep.task_id, 7);
        assert_eq!(dep.depends_on_id, 3);
    }

    #[test]
    fn task_note_fields_match_spec() {
        let note = TaskNote {
            id: Some(5),
            task_id: 7,
            content: "A note".to_string(),
            created_at: "2026-08-21T00:00:00Z".to_string(),
        };
        assert_eq!(note.id, Some(5));
        assert_eq!(note.task_id, 7);
        assert_eq!(note.content, "A note");
        assert_eq!(note.created_at, "2026-08-21T00:00:00Z");
    }

    #[test]
    fn task_note_serializes_and_deserializes() {
        let note = TaskNote {
            id: None,
            task_id: 2,
            content: "note content".to_string(),
            created_at: "now".to_string(),
        };
        let json = serde_json::to_string(&note).unwrap();
        let parsed: TaskNote = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, note);
    }

    #[test]
    fn task_note_clones_and_compares() {
        let note = TaskNote {
            id: Some(1),
            task_id: 2,
            content: "x".to_string(),
            created_at: "t".to_string(),
        };
        let clone = note.clone();
        assert_eq!(note, clone);
    }

    #[test]
    fn task_note_new_rejects_empty_content() {
        assert!(matches!(TaskNote::new(1, ""), Err(AppError::Validation(_))));
    }

    #[test]
    fn task_note_new_rejects_whitespace_only_content() {
        assert!(matches!(
            TaskNote::new(1, "  \t\n  "),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn task_note_new_accepts_valid_content() {
        let note = TaskNote::new(7, "a note").unwrap();
        assert_eq!(note.id, None);
        assert_eq!(note.task_id, 7);
        assert_eq!(note.content, "a note");
        assert_eq!(note.created_at, "");
    }

    #[test]
    fn status_and_priority_impl_json_schema() {
        assert_eq!(Status::schema_name(), "Status");
        assert_eq!(Priority::schema_name(), "Priority");
    }

    #[test]
    fn status_label_returns_human_readable_name() {
        assert_eq!(Status::Todo.label(), "Todo");
        assert_eq!(Status::InProgress.label(), "In Progress");
        assert_eq!(Status::Done.label(), "Done");
    }

    #[test]
    fn priority_label_returns_human_readable_name() {
        assert_eq!(Priority::Low.label(), "Low");
        assert_eq!(Priority::Medium.label(), "Medium");
        assert_eq!(Priority::High.label(), "High");
    }

    #[test]
    fn status_style_returns_colored_style() {
        assert_eq!(Status::Todo.style().fg, Some(Color::Yellow));
        assert_eq!(Status::InProgress.style().fg, Some(Color::Blue));
        assert_eq!(Status::Done.style().fg, Some(Color::Green));
    }

    #[test]
    fn priority_style_returns_colored_style() {
        assert_eq!(Priority::High.style().fg, Some(Color::Red));
        assert_eq!(Priority::Medium.style().fg, Some(Color::Yellow));
        assert_eq!(Priority::Low.style().fg, Some(Color::Green));
    }

    #[test]
    fn status_row_style_done_is_dark_gray() {
        assert_eq!(Status::Done.row_style().fg, Some(Color::DarkGray));
    }

    #[test]
    fn status_row_style_non_done_is_default() {
        assert_eq!(Status::Todo.row_style().fg, None);
        assert_eq!(Status::InProgress.row_style().fg, None);
    }
}
