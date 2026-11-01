---
name: writing-sql
description: Writing SQL in the distributed workspace — migrations, column formatting, schema patterns (domains, enums, triggers), and the sqlx migration lifecycle. Use when creating or editing files under crates/domains/*/migrations or hand-writing SQL in repositories.
---

# Writing SQL in the distributed workspace

Read `AGENTS.md` at the repo root first — it owns the migration commands table
(`just migrate <domain> …`) and the migration-history rule (never rewrite
applied migrations without the human stating the deployment situation).

## Placement and naming

- Migrations live in `crates/domains/<domain>/migrations/` — one `up.sql`/`down.sql`
  pair per migration, `<version>_<snake_case_action>.sql` (e.g.
  `20261028013924_create_table_idempotency_keys`).
- A standalone type definition gets its own migration *before* the one that uses it
  (precedent: `20261028010000_create_type_idempotency_key_status`,
  `20261013014954_create_domain_string`).
- Folded secondary objects (an index on the table just created) live in the same
  migration as their table; the down file drops children before the table.

## Column formatting — the alignment rule

In a `create table` column list: column names flush left, types aligned to the
longest type name, and **everything else** (`not null`, `default`, `check`)
starts at one fixed column — one space past the longest type's field.
`not null` must sit at the *same* column on every row; verify with `awk`, never
by eye. Check-constraint continuations align under the first character of their
expression. Commas stay adjacent to the type (`uuid,`) — the modifier column is
for modifiers, not commas (PR 66 ruling).

Model block (from the landed `idempotency_keys` migration):

```sql
create table idempotency_keys (
  idempotency_key string                 not null check (idempotency_key ~ '^[!-~]+$'
                                                         and length(idempotency_key) between 1 and 64),
  fingerprint     string                 not null,
  status          idempotency_key_status not null,
  entity_id       uuid,
  created_at      timestamptz(6)         not null default clock_timestamp(),
  primary key (idempotency_key),
  check (status = 'pending' or entity_id is not null)
);

select util.add_updated_at('idempotency_keys');

create index idempotency_keys_entity_id_idx on idempotency_keys (entity_id);
```

## Schema patterns

- Text columns use the `string` domain (non-empty check built in), not bare `text`.
- Enumeration-like columns use a Postgres enum type over a check constraint;
  reads map it with a `#[derive(sqlx::Type)]` Rust enum (`type_name` +
  `rename_all = "lowercase"`), writes cast text parameters (`CAST($n AS <enum>)`).
- `updated_at` maintenance goes through `util.add_updated_at('<table>')`
  (schema + trigger function are defined once in the domain's early migrations) —
  never hand-roll triggers per table.
- Invariants that span columns are table-level `check` constraints.
- Foreign keys only where cross-entity integrity is genuinely wanted; an index on
  the referencing column can replace one (precedent: `idempotency_keys.entity_id`
  dropped its FK in favor of `idempotency_keys_entity_id_idx`).

## Provisioning a new domain

Every domain owns its own database (provisioned in the docker setup), so each
domain's migration set must be **self-contained** — it re-creates the
database-global objects it needs rather than depending on another domain's
migrator having run first.

- The canonical copies of the shared objects live in
  `crates/libs/database/migrations/` (embedded via `database::MIGRATOR` for the
  database crate's own tests): schema `util`, domain `string`,
  `util.add_updated_at`, and (when the domain uses idempotency — the Rust
  machinery is in `libs/database`) the `idempotency_key_status` enum +
  `idempotency_keys` table. Existing domains keep their own self-contained
  copies (`todos` retains its own).
- Seed the new domain's `migrations/` by **copying** those files (fresh versions
  that sort before the domain's first table), never by referencing them.
  Collisions across domains are not a concern — separate databases means
  separate `_sqlx_migrations` tables.
- Omitting a shared object fails loudly at first apply (e.g. `create table`
  referencing a missing `string` domain), not silently — that failure is the
  reminder to seed.
- Then follow the placement rules above: standalone types before use, domain
  tables after the seeded shared set.

## Lifecycle (sqlx checksums)

- `just migrate <domain> run` validates the checksums of applied migrations and
  applies pending ones; a second run must be a silent no-op — that no-op is the
  gate that your files are consistent.
- Editing any applied migration changes its checksum, and later migrations
  cannot be applied in order past it. The only safe edit path on a dev database:
  `just migrate <domain> revert --target-version 0` → edit files → `run` →
  second `run` (no-op). Never hand-edit a checksum-relevant file without it.
- Verify schema expectations live via `just psql <database>` (`\d <table>` for
  FKs/indexes; deliberate bad-value inserts for constraints).
