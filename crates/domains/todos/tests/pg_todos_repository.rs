//! Integration tests for the [`PgTodosRepository`] adapter's timestamp binds
//! against a real Postgres database.

use sqlx::PgPool;

use database::Database;
use todos::models::todo::{NewTodoModel, TodoCursor, TodoFilter, TodoModel, TodoSortKey};
use todos::{PgTodosRepository, TodosRepository};
use utils::pagination::{Cursor, Sort, SortOrder};

#[must_use]
fn tasks_of(todos: &[TodoModel]) -> Vec<&str> {
    todos.iter().map(|t| t.task.as_str()).collect()
}

#[sqlx::test(migrator = "todos::MIGRATOR")]
async fn add_round_trips_a_completed_at(postgres: PgPool) {
    let repository = PgTodosRepository::new(Database::Pool(postgres));
    let new = NewTodoModel {
        task: "completed long ago".to_string(),
        completed_at: Some(jiff::Timestamp::UNIX_EPOCH),
    };

    let added = repository.add(&new).await.unwrap();
    assert_eq!(added.completed_at, Some(jiff::Timestamp::UNIX_EPOCH));

    let stored = repository
        .get_by_id(added.todo_id)
        .await
        .unwrap()
        .expect("added todo must be stored");
    assert_eq!(stored.completed_at, Some(jiff::Timestamp::UNIX_EPOCH));
}

#[sqlx::test(migrator = "todos::MIGRATOR")]
async fn seek_by_updated_at_asc_returns_only_later_todos(postgres: PgPool) {
    let repository = PgTodosRepository::new(Database::Pool(postgres));
    let tasks = ["first", "second", "third"];
    for task in tasks {
        repository
            .add(&NewTodoModel {
                task: task.to_string(),
                completed_at: None,
            })
            .await
            .unwrap();
    }

    let sort = Sort {
        key: TodoSortKey::UpdatedAt,
        order: SortOrder::Asc,
    };
    let page = repository
        .list_by_filter(&TodoFilter::default(), sort, &Cursor::default())
        .await
        .unwrap();
    assert_eq!(tasks_of(&page), tasks);

    let rest = repository
        .list_by_filter(
            &TodoFilter::default(),
            sort,
            &Cursor {
                after: Some(TodoCursor::from(&page[1])),
                limit: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(tasks_of(&rest), ["third"]);
}

#[sqlx::test(migrator = "todos::MIGRATOR")]
async fn seek_by_updated_at_desc_returns_only_earlier_todos(postgres: PgPool) {
    let repository = PgTodosRepository::new(Database::Pool(postgres));
    let tasks = ["first", "second", "third"];
    for task in tasks {
        repository
            .add(&NewTodoModel {
                task: task.to_string(),
                completed_at: None,
            })
            .await
            .unwrap();
    }

    let sort = Sort {
        key: TodoSortKey::UpdatedAt,
        order: SortOrder::Desc,
    };
    let page = repository
        .list_by_filter(&TodoFilter::default(), sort, &Cursor::default())
        .await
        .unwrap();
    assert_eq!(tasks_of(&page), ["third", "second", "first"]);

    let rest = repository
        .list_by_filter(
            &TodoFilter::default(),
            sort,
            &Cursor {
                after: Some(TodoCursor::from(&page[1])),
                limit: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(tasks_of(&rest), ["first"]);
}
