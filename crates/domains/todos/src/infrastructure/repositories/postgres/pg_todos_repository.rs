use async_trait::async_trait;
use futures::TryStreamExt;
use sea_query::extension::postgres::PgExpr;
use sea_query::{Alias, Expr, ExprTrait, Iden, Order, PostgresQueryBuilder, Query, SimpleExpr};
use sea_query_sqlx::SqlxBinder;
use sqlx::{self, AssertSqlSafe};
use tracing::instrument;
use tracing_futures::Instrument;
use uuid::Uuid;

use crate::models::todo::{
    NewTodoModel, TodoCursor, TodoFilter, TodoModel, TodoSortKey, TodoUpdateModel,
};
use crate::use_cases::repositories::{TodoStream, TodosRepository as Interface};
use database::{Database, DbContext};
use storage::StorageError;
use utils::pagination::{Cursor, Sort, SortOrder};

#[derive(Clone, Copy, Iden)]
enum Todos {
    Table,
    TodoId,
    Task,
    CompletedAt,
    UpdatedAt,
}

#[derive(sqlx::FromRow)]
struct TodoRow {
    todo_id: Uuid,
    task: String,
    completed_at: Option<jiff_sqlx::Timestamp>,
    updated_at: jiff_sqlx::Timestamp,
}

impl From<TodoRow> for TodoModel {
    fn from(row: TodoRow) -> Self {
        Self {
            todo_id: row.todo_id,
            task: row.task,
            completed_at: row.completed_at.map(jiff_sqlx::Timestamp::to_jiff),
            updated_at: row.updated_at.to_jiff(),
        }
    }
}

/// Postgres implementation of the
/// [`TodosRepository`](crate::use_cases::repositories::TodosRepository) port.
#[derive(Clone, Debug)]
pub struct PgTodosRepository {
    database: Database,
}

impl PgTodosRepository {
    /// Creates a repository backed by the given [`Database`] connection
    /// source.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }
}

#[async_trait]
impl Interface for PgTodosRepository {
    #[instrument(skip_all)]
    async fn add(&self, todo: &NewTodoModel) -> Result<TodoModel, StorageError> {
        let completed_at = todo.completed_at.map_or_else(Expr::null, |ts| {
            Expr::value(ts.to_string()).cast_as(Alias::new("timestamptz"))
        });
        let (text, values) = Query::insert()
            .into_table(Todos::Table)
            .columns([Todos::Task, Todos::CompletedAt])
            .values_panic([Expr::val(&todo.task), completed_at])
            .returning(Query::returning().columns([
                Todos::TodoId,
                Todos::Task,
                Todos::CompletedAt,
                Todos::UpdatedAt,
            ]))
            .build_sqlx(PostgresQueryBuilder);

        let mut conn = self.database.connect().await.map_err(StorageError::from)?;

        sqlx::query_as_with::<_, TodoRow, _>(AssertSqlSafe(text), values)
            .fetch_one(&mut *conn)
            .await
            .map(TodoModel::from)
            .db_context("inserting todo")
            .map_err(StorageError::from)
    }

    #[instrument(skip_all)]
    async fn update(
        &self,
        todo_id: Uuid,
        todo: &TodoUpdateModel,
    ) -> Result<Option<TodoModel>, StorageError> {
        let (text, values) = Query::update()
            .table(Todos::Table)
            .value(Todos::Task, Expr::val(&todo.task))
            .value(
                Todos::CompletedAt,
                Expr::case(
                    Expr::value(todo.is_completed),
                    Expr::cust("clock_timestamp()"),
                )
                .finally(Expr::null()),
            )
            .and_where(Expr::col(Todos::TodoId).eq(todo_id))
            .returning(Query::returning().columns([
                Todos::TodoId,
                Todos::Task,
                Todos::CompletedAt,
                Todos::UpdatedAt,
            ]))
            .build_sqlx(PostgresQueryBuilder);

        let mut conn = self.database.connect().await.map_err(StorageError::from)?;

        sqlx::query_as_with::<_, TodoRow, _>(AssertSqlSafe(text), values)
            .fetch_optional(&mut *conn)
            .await
            .map(|row| row.map(TodoModel::from))
            .db_context("updating todo")
            .map_err(StorageError::from)
    }

    #[instrument(skip_all)]
    async fn delete(&self, todo_id: Uuid) -> Result<bool, StorageError> {
        let (text, values) = Query::delete()
            .from_table(Todos::Table)
            .and_where(Expr::col(Todos::TodoId).eq(todo_id))
            .returning(Query::returning().columns([Todos::TodoId]))
            .build_sqlx(PostgresQueryBuilder);

        let mut conn = self.database.connect().await.map_err(StorageError::from)?;

        sqlx::query_scalar_with::<_, Uuid, _>(AssertSqlSafe(text), values)
            .fetch_optional(&mut *conn)
            .await
            .map(|deleted| deleted.is_some())
            .db_context("deleting todo")
            .map_err(StorageError::from)
    }

    fn stream_by_filter(
        &self,
        filter: &TodoFilter,
        sort: Sort<TodoSortKey>,
        cursor: &Cursor<TodoCursor>,
    ) -> TodoStream<'_> {
        let direction = match sort.order {
            SortOrder::Asc => Order::Asc,
            SortOrder::Desc => Order::Desc,
        };
        let sort_columns: &[Todos] = match sort.key {
            TodoSortKey::TodoId => &[Todos::TodoId],
            TodoSortKey::UpdatedAt => &[Todos::UpdatedAt, Todos::TodoId],
        };
        let (text, values) = Query::select()
            .columns([
                Todos::TodoId,
                Todos::Task,
                Todos::CompletedAt,
                Todos::UpdatedAt,
            ])
            .from(Todos::Table)
            .apply_if(filter.todo_id.as_deref(), |q, v| {
                q.and_where(Expr::col(Todos::TodoId).eq_any(v.iter().copied()));
            })
            .apply_if(filter.is_completed, |q, v| {
                q.and_where(if v {
                    Expr::col(Todos::CompletedAt).is_not_null()
                } else {
                    Expr::col(Todos::CompletedAt).is_null()
                });
            })
            .apply_if(cursor.after, |q, after| {
                q.and_where(cursor_predicate(sort.key, sort.order, after));
            })
            .order_by_columns(sort_columns.iter().map(|c| (*c, direction.clone())))
            .apply_if(cursor.limit, |q, v| {
                q.limit(v);
            })
            .build_sqlx(PostgresQueryBuilder);

        let database = self.database.clone();

        let stream = async_stream::try_stream! {
            let mut conn = database.connect().await.map_err(StorageError::from)?;
            let mut rows = sqlx::query_as_with::<_, TodoRow, _>(AssertSqlSafe(text), values)
                .fetch(&mut *conn);
            while let Some(row) = rows
                .try_next()
                .await
                .db_context("streaming todo")
                .map_err(StorageError::from)?
            {
                yield TodoModel::from(row);
            }
        };

        Box::pin(stream.instrument(tracing::Span::current()))
    }
}

/// Builds the seek predicate for `cursor.after` against the chosen sort key.
///
/// Invariant: the cursor tuple must match the sort key — by
/// [`TodoSortKey::TodoId`] only `todo_id` participates in the comparison; by
/// [`TodoSortKey::UpdatedAt`] the `(updated_at, todo_id)` tuple is compared
/// positionally so rows sharing an `updated_at` still have a total order.
fn cursor_predicate(sort_key: TodoSortKey, order: SortOrder, after: TodoCursor) -> SimpleExpr {
    let updated = Expr::tuple([Expr::col(Todos::UpdatedAt), Expr::col(Todos::TodoId)]);
    let position = Expr::tuple([
        Expr::value(after.updated_at.to_string()).cast_as(Alias::new("timestamptz")),
        Expr::value(after.todo_id),
    ]);
    match (sort_key, order) {
        (TodoSortKey::TodoId, SortOrder::Asc) => Expr::col(Todos::TodoId).gt(after.todo_id),
        (TodoSortKey::TodoId, SortOrder::Desc) => Expr::col(Todos::TodoId).lt(after.todo_id),
        (TodoSortKey::UpdatedAt, SortOrder::Asc) => updated.gt(position),
        (TodoSortKey::UpdatedAt, SortOrder::Desc) => updated.lt(position),
    }
}
