use std::borrow::Cow;
use std::fmt::Write as _;

use jiff::Timestamp;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FINGERPRINT_PREFIX: &str = "create-todo";
const FINGERPRINT_ABSENT_COMPLETED_AT: &str = "none";

/// Input for creating a new todo.
#[derive(Debug, Clone)]
pub struct NewTodoModel {
    /// Task description; valid when non-empty after trimming (see
    /// [`Self::has_valid_task`]).
    pub task: String,
    /// When the todo was completed, or [`Option::None`] while it is open.
    pub completed_at: Option<Timestamp>,
}

impl NewTodoModel {
    /// A task is valid when it contains at least one non-whitespace
    /// character after trimming.
    #[must_use]
    pub fn has_valid_task(&self) -> bool {
        !self.task.trim().is_empty()
    }

    /// Lowercase-hex SHA-256 fingerprint of the canonical request form used
    /// for idempotency comparisons.
    ///
    /// The canonical form is `create-todo|<task>|<completed_at>` where
    /// `completed_at` is the RFC 3339 timestamp, or `none` for an open todo.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let canonical_completed_at: Cow<'static, str> = self.completed_at.map_or_else(
            || Cow::Borrowed(FINGERPRINT_ABSENT_COMPLETED_AT),
            |ts| Cow::Owned(ts.to_string()),
        );
        let mut hasher = Sha256::new();
        hasher.update(format!(
            "{FINGERPRINT_PREFIX}|{}|{}",
            self.task, canonical_completed_at
        ));
        let digest = hasher.finalize();
        digest
            .iter()
            .fold(String::with_capacity(digest.len() * 2), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            })
    }
}

/// Full-replace update input for a todo.
#[derive(Debug, Clone)]
pub struct TodoUpdateModel {
    /// Replacement task description; valid when non-empty after trimming
    /// (see [`Self::has_valid_task`]).
    pub task: String,
    /// Desired completion state: `true` stamps the todo's completion time,
    /// `false` clears it.
    pub is_completed: bool,
}

impl TodoUpdateModel {
    /// A task is valid when it contains at least one non-whitespace
    /// character after trimming.
    #[must_use]
    pub fn has_valid_task(&self) -> bool {
        !self.task.trim().is_empty()
    }
}

/// A todo as stored by the repository.
#[derive(Debug)]
pub struct TodoModel {
    /// Unique identifier of the todo.
    pub todo_id: Uuid,
    /// Task description.
    pub task: String,
    /// When the todo was completed, or [`Option::None`] while it is open.
    pub completed_at: Option<Timestamp>,
    /// When the todo was last created or updated.
    pub updated_at: Timestamp,
}

/// Filter for listing todos. All fields are optional conjunctive filters;
/// `todo_id` is "any-of" semantics — when present, only todos whose id is in
/// the list are returned (an empty list matches nothing).
#[derive(Debug, Default)]
pub struct TodoFilter {
    /// Restricts results to todos whose id is in the list; [`Option::None`]
    /// matches any id.
    pub todo_id: Option<Vec<Uuid>>,
    /// Restricts results to todos in this completion state; [`Option::None`]
    /// matches both open and completed todos.
    pub is_completed: Option<bool>,
}

/// Sort key for listing todos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TodoSortKey {
    /// Order by todo id — the order in which todos were created.
    #[default]
    TodoId,
    /// Order by the time each todo was last updated.
    UpdatedAt,
}

/// Opaque pagination cursor position for a todo.
///
/// The `updated_at`/`todo_id` pair is the composite key used when sorting by
/// `TodoSortKey::UpdatedAt`; when sorting by `TodoSortKey::TodoId` only
/// `todo_id` participates in the seek predicate. The tuple must always
/// correspond to the sort key chosen in `stream_by_filter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TodoCursor {
    /// Id of the last todo on the previous page.
    pub todo_id: Uuid,
    /// Update time of the last todo on the previous page.
    pub updated_at: Timestamp,
}

impl From<&TodoModel> for TodoCursor {
    fn from(todo: &TodoModel) -> Self {
        Self {
            todo_id: todo.todo_id,
            updated_at: todo.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_pins_absent_completed_at() {
        let todo = NewTodoModel {
            task: "write the report".to_string(),
            completed_at: None,
        };

        assert_eq!(
            todo.fingerprint(),
            "e860340bfeee1a3e31d409c1fe9b72555e34ac0f609bb90f086a6f8a02e26048"
        );
    }

    #[test]
    fn fingerprint_pins_present_completed_at() {
        let todo = NewTodoModel {
            task: "write the report".to_string(),
            completed_at: Some(Timestamp::UNIX_EPOCH),
        };

        assert_eq!(
            todo.fingerprint(),
            "78b9c0a78661059024340e3fe8e825e2a81905006f14d4f06ea55306885df9b4"
        );
    }

    #[test]
    fn fingerprint_is_stable_across_calls() {
        let todo = NewTodoModel {
            task: "write the report".to_string(),
            completed_at: None,
        };

        assert_eq!(todo.fingerprint(), todo.fingerprint());
    }

    #[test]
    fn fingerprint_distinguishes_tasks() {
        let todo = NewTodoModel {
            task: "write the report".to_string(),
            completed_at: None,
        };
        let other = NewTodoModel {
            task: "read the report".to_string(),
            completed_at: None,
        };

        assert_ne!(todo.fingerprint(), other.fingerprint());
    }

    #[test]
    fn fingerprint_distinguishes_completed_at_values() {
        let todo = NewTodoModel {
            task: "write the report".to_string(),
            completed_at: None,
        };
        let other = NewTodoModel {
            task: "write the report".to_string(),
            completed_at: Some(Timestamp::UNIX_EPOCH),
        };

        assert_ne!(todo.fingerprint(), other.fingerprint());
    }

    #[test]
    fn fingerprint_is_64_char_lowercase_hex() {
        let todo = NewTodoModel {
            task: "write the report".to_string(),
            completed_at: None,
        };
        let fingerprint = todo.fingerprint();

        assert_eq!(fingerprint.len(), 64);
        assert!(
            fingerprint
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "expected lowercase hex, got {fingerprint}"
        );
    }
}
