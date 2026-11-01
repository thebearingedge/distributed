# database

Shared persistence plumbing for the distributed workspace: the generic transaction
unit-of-work, the idempotency-key machinery, and the database error taxonomy.
Apps → domains → libs: domains speak this crate's ports; it knows nothing about
any specific domain.

## Contents

- [`Database` / `UnitOfWork` / `Conn`](src/unit_of_work.rs) — transaction
  source; domain unit-of-work ports delegate to these.
- [Idempotency](src/idempotency.rs) — `IdempotencyRepository` port,
  `PgIdempotencyRepository` adapter, and key/claim primitives for
  idempotent request handling.
- [`DbError` / `DbContext`](src/db_error.rs) — classified database errors and
  pool context.
- [`MIGRATOR`](src/lib.rs) — embeds the canonical base migrations in
  [migrations/](migrations/), used by this crate's own tests.

## Migrations policy

Each domain owns a self-contained migration set for its own database (provisioned
per domain in the docker setup). The database-global SQL objects every domain
needs — schema `util`, domain `string`, `util.add_updated_at`, and the
idempotency enum + table — have their canonical copies in this crate's
[migrations/](migrations/), embedded via `database::MIGRATOR`. Domains keep
their own self-contained copies of those files (`todos` retains its own set);
when creating a new domain, seed its `migrations/` with copies from this crate.
See the provisioning section of the `writing-sql` skill.
