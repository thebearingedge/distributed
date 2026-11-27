//! The route paths of the todos API, shared by the router in [`api`] and the
//! `Location` header of the create response in [`controller`].

/// The todos collection path: the `POST` create route and the `GET` list
/// route.
pub const TODOS_PATH: &str = "/v1/todos";

/// The single-todo path template: the `GET`, `PUT`, and `DELETE` routes for
/// one todo, and the prefix of the `Location` header on the create response.
pub const TODO_PATH: &str = "/v1/todos/{todo_id}";
