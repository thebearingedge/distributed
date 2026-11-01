---
name: writing-rust
description: Writing or modifying Rust code in the distributed workspace. Use when creating or editing .rs files, Cargo.toml members, migrations, or tests — covers Clean Architecture layering, error handling patterns, module layout, and the lint/test loop.
---

# Writing Rust in the distributed workspace

Read `AGENTS.md` at the repo root first — it holds the hard rules (commit policy, branch naming, secrets, dependency policy, scratch locations).

Adding a new HTTP endpoint to an API app? Follow the `adding-an-endpoint` skill — it expands the inside-out workflow below into the endpoint-specific playbook (DTO naming, OpenAPI wiring, validation, error mapping, per-handler test layout).

## Style priorities

Two qualities carry extra weight in review, alongside the rules below:

- **Idiomatic over manual:** prefer the construct the language/std offers —
  iterator adapters and combinators over `if`-based `Option`/`Result`
  plumbing, `match` over `if/else` on `bool` predicates, method references over
  trivial closures. When a helper only *reads* a value it doesn't own, take it
  by reference; take it by value only when it consumes it.
- **Typed over stringly-typed:** pass typed values (`Method`, enums, newtypes)
  through APIs instead of string parameters; keep statuses, modes, and
  identifiers as enum variants or named constants and produce their string form
  only at the boundary — serialization (`serde` renames), header construction,
  wire formats. Never compare or branch on bare string literals for state.

## Placement — decide the layer before writing code

Dependency direction is strictly apps → domains → libs. Put new code where its dependencies point outward:

- **HTTP/framework/CLI concerns** (axum handlers, extractors, config parsing, `main`) → `crates/apps/<category>/<app>` (e.g. `api/todos-api`, `worker/email-worker`, `cli/ops-cli`)
- **Business rules, ports, use cases** → `crates/domains/<domain>/src/use_cases`
- **Domain models and invariants** (validation like `has_valid_task`) → `crates/domains/<domain>/src/models`
- **Database implementations** (sqlx, sea-query) → `crates/domains/<domain>/src/infrastructure`
- **Generic reusable plumbing** (web middleware, error envelopes, telemetry, pagination) → `crates/libs/*`. Code with cross-domain shape (ports, models, adapters a second domain could adopt) is extracted to the appropriate lib — or a new lib crate — per the extraction policy in `AGENTS.md`; don't let it strand in one domain.

Leaks to avoid (found in past reviews):

- No sqlx/postgres types in use_cases — use case code sees only its repository trait.
- No domain-specific types in shared libs — e.g. `CursorError::SortMismatch` existing only for todos in `libs/web` was flagged; add a config knob or keep the concern in the app instead.
- Domain → HTTP status mapping belongs in the app's `app_error.rs`, never in `libs/web` (`From<StorageError>` impls hard-coding 503/409/500 there were flagged).
- No UUID-version or ID-scheme checks in use cases (`get_version_num() != 7` conflated "bad ID" with "not found" — don't repeat it).
- Domain-general machinery stranded in a single domain (e.g. the idempotency machinery initially built in `todos`) gets flagged for extraction.

## Naming and structure — suffixes carry the layer

- **Domain models** (in `crates/domains/<domain>/src/models`) end in `Model`: `TodoModel`, `NewTodoModel`, `TodoUpdateModel`.
- **Port files** in `crates/domains/<domain>/src/use_cases` are entity-qualified: `todo_unit_of_work.rs` — a domain may contain multiple entities, so port files anchor on their entity (singular), not the domain name.
- **Adapter implementation files** inside a backend adapter directory (`infrastructure/repositories/postgres/`) are backend-prefixed: `pg_todos_repository.rs`, `pg_unit_of_work.rs` — this disambiguates them from same-named port files and mirrors the `Pg*` type names. Mods match file names; external public paths are unaffected (re-exports keep the type names at the same paths).
- **Shared-crate consts are concept-prefixed**, not generic: `IDEMPOTENCY_MIN_LENGTH`, not `MIN_LENGTH` — generic names collide at re-export sites (PR 66 review).
- HTTP-side naming and placement (DTOs mirroring `Model` names as `Http`, `Query`/path structs, colocating structs with their handler) is endpoint-specific — see the `adding-an-endpoint` skill.

## Comments — what belongs in code

- **Rustdoc is expected and lint-enforced**: the workspace sets `missing_docs = "warn"` (`[workspace.lints.rust]` in the root `Cargo.toml`, inherited by every crate) and CI fails on warnings, so every public item — including fields, enum variants, trait methods, and binary/test crate roots — needs a doc comment. Never add `#[allow(missing_docs)]`; document the item instead. Keep `# Errors` sections on fallible functions and `# Panics` on panicking ones. Intra-doc links on items visible at a crate root must be crate-qualified (`` [`storage::StorageError`] `` style, or `` [`crate::…`] ``) — docs on re-exported items re-render at crate-root scope, where short links silently break; verify with `cargo doc --no-deps -p <crate>`.
- **No "decision" comments**: never narrate choices, history, or rationale in code ("agreed rule:", "deliberately untouched", "mirrors the existing pattern", "we chose X because Y"). If a decision needs recording, it goes in the plan/notes, not the source.
- **No third-party API documentation comments**: never paraphrase what a framework, crate, or database feature does (e.g. what `AssertSqlSafe` means, what a Postgres function returns, how sea-query renders an expression). The code and upstream docs cover it.
- **Explanatory comments are opt-in**: anything beyond rustdoc must be explicitly planned and approved ahead of time (list the comment in the plan task's `Work:` section). Treat any unplanned explanatory comment as a review finding.

## Patterns

- **Imports**: three groups separated by blank lines, in this order — std library, third-party crates, then one contiguous block of internal workspace crates (`todos`, `database`, `telemetry`, `storage`, `utils`, `web`) together with the current crate's paths (`crate::` / `super::`) — project crate imports don't need separation lines (PR 66 review). rustfmt orders `super::`/`crate::` within the merged block, so they sort first — that is expected, not a formatting bug. The same rule applies inside `#[cfg(test)]` modules and to integration-test files under `crates/**/tests`.
- **Error types**: one dedicated file per crate/layer named `<thing>_error.rs`, using `thiserror`. When wrapping a source error, interpolate `{source}` in the `#[error(...)]` message so diagnostics survive `Display` — this binds only for fields *named* `source`; for unnamed `#[source]` tuple fields, positional `{0}` is the correct equivalent (thiserror 2.x does not bind `{source}` for tuple fields; verified in review of issue #70). If a struct-variant split (like `DbError` → `StorageError` classification) is intentional, document it at the top of the file. (The app-layer HTTP error envelope convention is endpoint-specific — see `adding-an-endpoint`.)
- **Modules**: multi-file modules use the file-and-folder pattern — `mod.rs`-style parent file (`repositories.rs`) alongside its directory (`repositories/todos_repository.rs`). New sibling modules should match.
- **Validation**: single source of truth. If a domain model validates (e.g. min length), don't re-encode the rule in the delivery layer (or make them agree); error messages must match the actual rule (`EmptyTask` shouldn't be raised when the real rule is "more than one character").
- **Dependencies**: workspace deps come from the root `Cargo.toml` `[workspace.dependencies]` — reference with `workspace = true`. Before proposing a new third-party dependency, check that no existing workspace dependency already covers the need. Looking up the latest version of a crate (crates.io/docs.rs) is fine, but never add or upgrade a dependency without explicit human permission — propose the crate, version, and reason first, then wait. Never declare a dep a crate doesn't use (unused `uuid`, `thiserror`, `axum` were all flagged).
- **Magic numbers** in shared plumbing (timeouts, limits): accept them as parameters or name them as constants — no bare literals with no configuration path.
- **`#[sqlx::test]` migrations**: add `migrator = "<domain>::MIGRATOR"` (plus `fixtures(...)`) only when the test executes SQL against that domain's schema — route handlers that query or write, repository tests, anything touching the tables. Tests that never run SQL — OpenAPI document checks, 404/405 fallback behavior, middleware/request-id plumbing, pool-failure paths — use a plain `#[sqlx::test]` (a fresh, empty database) so migration time isn't paid on every run. When in doubt, trace the route: if the handler's success and failure paths both leave the pool untouched, the schema isn't needed.

## Configuration

- **Layered lib configs**: reusable config structs live in libs (`web::config::ApiConfig`, `database::DbConfig`) and derive `Debug, clap::Parser` — not bare `clap::Args`, which clap 4.6's derive lacks `try_parse_from` on; the `Parser` derive also generates the `Args` impl, so `#[command(flatten)]` still consumes it. Every field pairs `#[arg(long, env = "…", …)]` with a doc comment (`missing_docs`) and a declarative default (`default_value` / `default_value_t`); env names are concept-prefixed (`DB_MIN_CONNECTIONS`, not `MIN_CONNECTIONS`). Typed accessors convert to std types (`Duration`, `Vec<HeaderValue>`) instead of leaking raw fields into wiring.
- **App composition root**: each app binary has a bin-private `src/config.rs` declared with `mod config;` in `main.rs` — a binary-target module, never exported from the lib target of lib+bin crates. It defines `pub(crate) struct Args` deriving `clap::Parser` and flattens exactly the lib configs the app uses:

  ```rust
  /// Command-line configuration for the todos API binary.
  #[derive(Debug, clap::Parser)]
  pub(crate) struct Args {
      /// Web/runtime configuration.
      #[command(flatten)]
      pub(crate) api: ApiConfig,
      /// Database configuration.
      #[command(flatten)]
      pub(crate) db: DbConfig,
  }
  ```

  Unparsable config fails fast via clap; no warn-and-default fallbacks.
- **`main.rs` is orchestration-only**: parse once at the top (`Args::parse()`), then only wire — no `std::env::var`, no hand-rolled parsing, no consts standing in for tunable knobs, no config-shaped types defined inline.
- **Composition at the app layer**: the merge point is the app's `Args` because only the app knows which libs it uses; libs must not depend on each other to unify config (no `web → database`), and apps must not advertise flags for database concerns they don't have (`default-api` flattens no database config).

## Before you finish

Run from the repo root (tooling needs direnv active — see `AGENTS.md`, whose
`## Commands` table owns the recipe set). The loop for a finished task:

1. `just cargo-fmt` — zero fmt drift; fix with `just cargo-fmt-fix`.
2. `just cargo-lint` — zero warnings. Mid-task, when the workspace is
   intentionally red (e.g. a trait method awaiting its impl), scope it:
   `just cargo-lint <crate>`; between designed intermediate states, plain
   `just cargo-check <crate>` is the fast loop.
3. `just cargo-test <crate>` for every crate you touched; integration tests
   need Postgres up: `just infrastructure up -d --wait`.
4. Migrations changed: `just migrate <domain> run` (revert syntax lives in
   the `AGENTS.md` commands table).

Prefer the `just` recipes over bare `cargo` commands — they wire in the
per-crate env files (`tools/envs/<name>.env`) and keep one place for flags.

Commits are governed by the `AGENTS.md` hard rules (explicit human permission; the commit is performed by the human or the built-in `build` agent). `.agents/temp/` is your scratch/output area — put notes, plans, and generated artifacts there (forbidden locations: see `AGENTS.md`).
