use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{app_error::AppError, app_state::AppState, paths::TODOS_PATH};
use database::{
    IDEMPOTENCY_MAX_LENGTH, IDEMPOTENCY_MIN_LENGTH, IDEMPOTENCY_VISIBLE_ASCII_PATTERN,
    IdempotencyKey,
};
use todos::models::todo::{
    NewTodoModel, TodoCursor, TodoFilter, TodoModel, TodoSortKey, TodoUpdateModel,
};
use todos::{TodoCreation, TodosInteractor};
use utils::pagination::{Cursor, Page, Sort, SortOrder};
use web::{
    cursor_token::CursorError,
    extractors::{HeaderConfig, OptionalHeader, ValidatedJson, ValidatedPath, ValidatedQuery},
};

/// The request body for creating a new todo.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewTodoHttp {
    /// The task text. Must contain at least one non-whitespace character
    /// after trimming.
    #[validate(length(min = 1))]
    task: String,
}

impl From<NewTodoHttp> for NewTodoModel {
    fn from(new: NewTodoHttp) -> Self {
        NewTodoModel {
            task: new.task,
            completed_at: None,
        }
    }
}

/// A todo as returned by every operation that produces one — the HTTP
/// representation of [`TodoModel`].
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TodoHttp {
    /// The unique identifier of the todo.
    pub todo_id: Uuid,
    /// The task text.
    pub task: String,
    /// When the todo was last created or updated.
    pub updated_at: Timestamp,
    /// When the todo was completed, or `null` while it is still open.
    pub completed_at: Option<Timestamp>,
}

impl From<TodoModel> for TodoHttp {
    fn from(model: TodoModel) -> Self {
        TodoHttp {
            todo_id: model.todo_id,
            task: model.task,
            updated_at: model.updated_at,
            completed_at: model.completed_at,
        }
    }
}

const IDEMPOTENT_REPLAYED_HEADER: &str = "idempotent-replayed";

/// Configuration for the optional `Idempotency-Key` request header of
/// [`add_todo`], extracted by [`OptionalHeader`]: absent means non-idempotent
/// creation, present means idempotent creation under that key.
///
/// A header that is present but not 1 to 64 visible ASCII characters rejects
/// the request with a `400 Bad Request` error body reading
/// [`HeaderConfig::REJECTION`].
pub struct IdempotencyKeyHeader;

impl HeaderConfig for IdempotencyKeyHeader {
    const HEADER: &'static str = "idempotency-key";

    const MIN_LENGTH: usize = IDEMPOTENCY_MIN_LENGTH;

    const MAX_LENGTH: usize = IDEMPOTENCY_MAX_LENGTH;

    const PATTERN: &'static str = IDEMPOTENCY_VISIBLE_ASCII_PATTERN;

    const REJECTION: &'static str = "invalid idempotency key";

    const DESCRIPTION: &'static str = "Optional client-supplied idempotency key: visible ASCII, \
         1 to 64 characters. Retrying with the same key and body replays the original todo; the \
         same key with a different body is rejected, and a key still held by an unfinished \
         request is rejected until that request completes. A key that is not valid is rejected \
         with `400 Bad Request`.";
}

/// Adds a new todo and returns it with a `201 Created` status.
///
/// The response carries a `Location` header with the relative path of the
/// new todo, built from [`crate::paths::TODOS_PATH`].
///
/// When the request carries an `Idempotency-Key` header, the creation is
/// idempotent: the same key and body replay the todo from the original
/// request, marked with an `Idempotent-Replayed: true` response header; the
/// same key with a different request body is rejected; and a key still held
/// by an unfinished request is rejected until that request completes.
/// Without the header, every request creates a new todo.
///
/// # Errors
///
/// Returns `AppError` as a `400 Bad Request` if the task is empty (an
/// invalid `Idempotency-Key` header is rejected by the [`OptionalHeader`]
/// extractor instead), as a `409 Conflict` if the
/// key is still held by an unfinished request, as a
/// `422 Unprocessable Entity` if the key was already used for a different
/// request body, or as a `500 Internal Server Error` /
/// `503 Service Unavailable` if storage fails.
pub async fn add_todo(
    State(app_state): State<AppState>,
    OptionalHeader { value: raw_key, .. }: OptionalHeader<IdempotencyKeyHeader>,
    ValidatedJson(new_todo): ValidatedJson<NewTodoHttp>,
) -> Result<Response, AppError> {
    let key = raw_key.as_deref().and_then(IdempotencyKey::parse);
    let TodoCreation { todo, replayed } = match key {
        Some(key) => {
            app_state
                .interactors
                .todos
                .add_idempotent(&key, &new_todo.into())
                .await?
        }
        None => TodoCreation {
            todo: app_state.interactors.todos.add(&new_todo.into()).await?,
            replayed: false,
        },
    };
    let mut response = (
        StatusCode::CREATED,
        [(header::LOCATION, format!("{TODOS_PATH}/{}", todo.todo_id))],
        Json(TodoHttp::from(todo)),
    )
        .into_response();
    if replayed {
        response
            .headers_mut()
            .insert(IDEMPOTENT_REPLAYED_HEADER, HeaderValue::from_static("true"));
    }
    Ok(response)
}

/// The property that todos are sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TodoSortKeyHttp {
    /// Sort by todo identifier.
    TodoId,
    /// Sort by last update time.
    UpdatedAt,
}

/// The direction in which todos are sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SortOrderHttp {
    /// Ascending order.
    Asc,
    /// Descending order.
    Desc,
}

impl From<TodoSortKeyHttp> for TodoSortKey {
    fn from(key: TodoSortKeyHttp) -> Self {
        match key {
            TodoSortKeyHttp::TodoId => Self::TodoId,
            TodoSortKeyHttp::UpdatedAt => Self::UpdatedAt,
        }
    }
}

impl From<TodoSortKey> for TodoSortKeyHttp {
    fn from(key: TodoSortKey) -> Self {
        match key {
            TodoSortKey::TodoId => Self::TodoId,
            TodoSortKey::UpdatedAt => Self::UpdatedAt,
        }
    }
}

impl From<SortOrderHttp> for SortOrder {
    fn from(order: SortOrderHttp) -> Self {
        match order {
            SortOrderHttp::Asc => Self::Asc,
            SortOrderHttp::Desc => Self::Desc,
        }
    }
}

impl From<SortOrder> for SortOrderHttp {
    fn from(order: SortOrder) -> Self {
        match order {
            SortOrder::Asc => Self::Asc,
            SortOrder::Desc => Self::Desc,
        }
    }
}

#[derive(Deserialize, Serialize)]
struct CursorToken {
    sort: TodoSortKeyHttp,
    order: SortOrderHttp,
    todo_id: Uuid,
    updated_at_micros: i64,
}

/// Query parameters parsed from the list request, ready to hand to the
/// interactor.
pub struct ListParams {
    /// Matches the `todo_id`/`is_completed` query parameters; `None` filters
    /// are unrestricted.
    pub filter: TodoFilter,
    /// The effective sort key: explicit from the query, otherwise carried
    /// over from the cursor token so pages sort consistently.
    pub sort: Sort<TodoSortKey>,
    /// The decoded cursor position and page size.
    pub cursor: Cursor<TodoCursor>,
}

impl TryFrom<ListTodosQuery> for ListParams {
    type Error = CursorError;

    fn try_from(query: ListTodosQuery) -> Result<Self, CursorError> {
        let token = query
            .cursor
            .as_deref()
            .map(web::cursor_token::decode::<CursorToken>)
            .transpose()?;
        let key = query
            .sort
            .or(token.as_ref().map(|t| t.sort))
            .unwrap_or_else(|| TodoSortKey::default().into());
        let order = query
            .order
            .or(token.as_ref().map(|t| t.order))
            .unwrap_or_else(|| SortOrder::default().into());
        let after = match token {
            Some(t) if t.sort != key || t.order != order => return Err(CursorError::SortMismatch),
            Some(t) => Some(TodoCursor {
                todo_id: t.todo_id,
                updated_at: Timestamp::from_microsecond(t.updated_at_micros)
                    .ok()
                    .ok_or(CursorError::Malformed)?,
            }),
            None => None,
        };

        Ok(Self {
            filter: TodoFilter {
                todo_id: query.todo_id,
                is_completed: query.is_completed,
            },
            sort: Sort {
                key: key.into(),
                order: order.into(),
            },
            cursor: Cursor {
                after,
                limit: query.limit,
            },
        })
    }
}

/// A page of todos returned by the list operation.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TodoPageHttp {
    /// The todos in this page, in the requested sort order.
    pub items: Vec<TodoHttp>,
    /// An opaque cursor token for fetching the next page, or `null` when this
    /// is the last page. Pass it as the `cursor` query parameter of the
    /// follow-up request.
    pub next: Option<String>,
}

impl TodoPageHttp {
    /// Builds the response body from the interactor's page, encoding the
    /// next-page cursor with the sort used for this request.
    #[must_use]
    pub fn new(page: Page<TodoModel, TodoCursor>, sort: Sort<TodoSortKey>) -> Self {
        let next = page.next.map(|cursor| {
            web::cursor_token::encode(&CursorToken {
                sort: sort.key.into(),
                order: sort.order.into(),
                todo_id: cursor.todo_id,
                updated_at_micros: cursor.updated_at.as_microsecond(),
            })
        });
        Self {
            items: page.items.into_iter().map(TodoHttp::from).collect(),
            next,
        }
    }
}

/// The maximum number of identifiers the `todoId` filter accepts.
pub const MAX_TODO_ID_FILTER_IDS: usize = 50;

/// The query parameters for listing todos. All parameters are optional; the
/// `todoId` and `isCompleted` filters combine conjunctively.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTodosQuery {
    /// Restricts results to todos whose identifier is in this list (an empty
    /// list matches nothing). At most [`MAX_TODO_ID_FILTER_IDS`] identifiers.
    #[validate(length(max = 50))]
    todo_id: Option<Vec<Uuid>>,
    /// Restricts results to todos that are completed (`true`) or open
    /// (`false`).
    is_completed: Option<bool>,
    /// The property to sort by. Defaults to `todoId`.
    sort: Option<TodoSortKeyHttp>,
    /// The direction to sort in. Defaults to `asc`.
    order: Option<SortOrderHttp>,
    /// The maximum number of todos to return, between 1 and
    /// [`TodosInteractor::MAX_PAGE_SIZE`].
    #[validate(range(min = 1, max = TodosInteractor::MAX_PAGE_SIZE))]
    limit: Option<u64>,
    /// An opaque cursor token from a previous page's `next` field, for
    /// fetching the next page. Its sort key and order must match `sort` and
    /// `order` when both are given.
    cursor: Option<String>,
}

/// Lists todos by filter with cursor-based pagination.
///
/// # Errors
///
/// Returns `AppError` as a `400 Bad Request` if the query fails validation
/// or the cursor token is *malformed* (base64/JSON decode failure), as a
/// `422 Unprocessable Entity` if the cursor is *invalid* (decodes but its
/// sort/order does not match the request), or as a
/// `500 Internal Server Error` if storage fails.
pub async fn list_todos(
    State(app_state): State<AppState>,
    ValidatedQuery(query): ValidatedQuery<ListTodosQuery>,
) -> Result<Json<TodoPageHttp>, AppError> {
    let ListParams {
        filter,
        sort,
        cursor,
    } = ListParams::try_from(query)?;
    let page = app_state
        .interactors
        .todos
        .list_by_filter(&filter, sort, &cursor)
        .await?;
    Ok(Json(TodoPageHttp::new(page, sort)))
}

/// The path parameters identifying a single todo.
#[derive(Deserialize, Serialize, JsonSchema)]
pub struct TodoPath {
    /// The unique identifier of the todo.
    todo_id: Uuid,
}

/// Gets a single todo by its ID.
///
/// # Errors
///
/// Returns `AppError` as a `404 Not Found` if the todo doesn't exist,
/// or a `500 Internal Server Error` if storage fails.
pub async fn get_todo(
    State(app_state): State<AppState>,
    ValidatedPath(path): ValidatedPath<TodoPath>,
) -> Result<Json<TodoHttp>, AppError> {
    let todo = app_state.interactors.todos.get_by_id(path.todo_id).await?;
    Ok(Json(todo.into()))
}

/// The request body for fully replacing a todo's mutable fields — mirrors
/// [`TodoUpdateModel`].
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TodoUpdateHttp {
    /// The new task text. Must contain at least one non-whitespace character
    /// after trimming.
    #[validate(length(min = 1))]
    task: String,
    /// The new completion state: `true` marks the todo completed.
    is_completed: bool,
}

impl From<TodoUpdateHttp> for TodoUpdateModel {
    fn from(update: TodoUpdateHttp) -> Self {
        TodoUpdateModel {
            task: update.task,
            is_completed: update.is_completed,
        }
    }
}

/// Fully replaces a todo's mutable fields and returns the updated todo.
///
/// # Errors
///
/// Returns `AppError` as a `400 Bad Request` if the task is empty,
/// a `404 Not Found` if the todo doesn't exist, or a
/// `500 Internal Server Error` / `503 Service Unavailable` on storage
/// failure.
pub async fn update_todo(
    State(app_state): State<AppState>,
    ValidatedPath(path): ValidatedPath<TodoPath>,
    ValidatedJson(update): ValidatedJson<TodoUpdateHttp>,
) -> Result<Json<TodoHttp>, AppError> {
    let todo = app_state
        .interactors
        .todos
        .update(path.todo_id, &update.into())
        .await?;
    Ok(Json(todo.into()))
}

/// Deletes a todo by its ID.
///
/// # Errors
///
/// Returns `AppError` as a `404 Not Found` if the todo doesn't exist, or
/// a `500 Internal Server Error` / `503 Service Unavailable` on storage
/// failure.
pub async fn delete_todo(
    State(app_state): State<AppState>,
    ValidatedPath(path): ValidatedPath<TodoPath>,
) -> Result<StatusCode, AppError> {
    app_state.interactors.todos.remove(path.todo_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
