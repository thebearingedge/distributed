# AGENTS.md

## Project

`distributed` is a monorepo for distributed systems. Backend code is currently Rust — an edition 2024 workspace organized with Clean Architecture layering; Go and JavaScript may be introduced later. Three layers with dependency direction apps → domains → libs:

- `crates/apps/` — deployables, at `crates/apps/<category>/<app>`: `api/todos-api` (axum HTTP API), `worker/email-worker`, `cli/ops-cli`
- `crates/domains/` — business logic (e.g. `todos`): use_cases define repository ports, infrastructure implements them (sqlx + sea-query), migrations live in `crates/domains/<domain>/migrations`
- `crates/libs/` — shared libraries: `web`, `database`, `storage`, `telemetry`, `utils`

Stack: axum, sqlx (Postgres), sea-query, opentelemetry, tracing, thiserror.

## Hard rules

- Never commit without explicit human permission. The commit itself is performed by the human or by the built-in `build` agent — the only agent configured with `ask` on commit (see `opencode.jsonc`).
- Never commit to the `main` branch. Commit only to branches named `issue-N/short-description`, where `N` is a GitHub issue number supplied by the human.
- No secrets may ever be put on the file system — not in the repo, not in `.agents/temp/`, not in env files or scratch files, anywhere. The reverse is explicitly fine: env files (`tools/envs/*.env`, `.envrc`) hold dev credentials, not secrets, and agents may read them freely without asking.
- Never add or upgrade a dependency without explicit human permission; looking up the latest version is fine.
- Scratch/output lives in `.agents/temp/` (gitignored) and never in `/tmp`, the repo root, `src/`, `coverage/`, or `target/`. Worktrees stay under `.agents/temp/`.
- Never operate outside the project root.
- Plan artifacts always live in `.agents/temp/issue-N-short-description/` (plans, phase files, notes, reports, reviews). This is official project policy and overrides any session- or harness-level plan-mode default pointing elsewhere (e.g. `~/.opencode/plan`).

## Commands

Tooling is managed via `direnv` (`.envrc` sets `CARGO_INSTALL_ROOT=tools/cargo` and adds `tools/cargo/bin` to `PATH`); run `just init` to install sqlx-cli, watchexec, and cargo-llvm-cov there. Additional CLIs are declared in `tools/Brewfile` and installed by a human via Homebrew — assume the tools listed there are available; never install or change tooling yourself.

| Task | Command |
|---|---|
| Check one crate | `just cargo-check <crate>` |
| Format check (fix) | `just cargo-fmt` (`just cargo-fmt-fix`) |
| Lint | `just cargo-lint` (clippy, `-D warnings`; per-crate: `just cargo-lint <crate>`) |
| Test one crate | `just cargo-test <crate>` |
| Test all | `just cargo-test-all` |
| Coverage | `just cargo-cover` (HTML + lcov in `coverage/`) |
| Run an app with hot reload | `just cargo-run <app> [args]` — valid `<app>` names are the crate names found in `crates/apps/**/Cargo.toml` (currently `todos-api`, `email-worker`, `ops-cli`) |
| TDD loop | `just cargo-tdd <crate>` |
| Migrations | `just migrate <domain> <action>` — the action passes through to `sqlx migrate` (`add` to create, `run` to apply, `revert` to undo, `revert --target-version 0` to revert all) |
| Postgres shell | `just psql <database>` |
| Compose up/down | `just infrastructure up -d` / `down` |

`just` recipes source env vars from `tools/envs/<name>.env` when the file exists (e.g. `todos-api.env`, `test.env`); crates without env files run plain — don't add empty env files as ceremony. CI (`.github/workflows/ci.yaml`) runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked` against docker-compose services — so keep formatting clean and pass `--locked` semantics (don't hand-edit `Cargo.lock`).

## Infrastructure

`infrastructure/docker-compose.yaml` provides local dev services:

- **postgres** (18, tmpfs storage) on `127.0.0.1:5432` — dev credentials only, not secrets
- **pgweb** UI on `127.0.0.1:8081`
- **opentelemetry-collector** on `4317`/`4318` (gRPC/HTTP)

Tests require the compose services running: `just infrastructure up -d --wait`.

## Conventions

- **Extraction policy:** any code that could be used by more than one domain
  must not live in a single domain crate. Extract it aggressively into the
  most appropriate `crates/libs/*` crate — or a brand-new lib crate when none
  fits — rather than letting shared-shaped code accumulate inside a domain.
  Applies when writing new code and to every refactoring pass after feature
  work lands.
- Agent skills live in `.agents/skills/<name>/SKILL.md`. When a session establishes new durable rules (lints, patterns, conventions), update the relevant skill so they carry forward. Layer and naming conventions, error-handling patterns, and Rust lint rules live in the `writing-rust` skill; SQL and migration conventions live in the `writing-sql` skill — not here.

## Agent Workflow

How agent sessions run — orchestration only. Code-level workflows live in the skills; commands live in `## Commands`.

**Session loop:**

1. The human names the issue: number + short-description.
2. The plan agent writes `.agents/temp/issue-N-short-description/plan.md`; explore subagents produce `notes-<topic>.md` alongside it.
3. The human reviews the plan.
4. The parent launches a coder per task (inputs: plan path, phase file, task id).
5. The coder writes `report-<task-id>.md` in the issue dir.
6. The reviewer reviews and writes `review-<task-id>.md`.
7. Verdict routing follows the `## Routing` section of `.opencode/agents/reviewer.md`.
8. When all tasks pass, the human approves and the commit happens.

**Approval gate:** the human approves the plan before any coder runs and approves each commit. Agents propose patches; humans execute or authorize git operations — commits happen only via the built-in `build` agent's `ask` path (see `opencode.jsonc`) or by the human directly.

**Refactoring:** for convention-debt cleanups (misplaced constants, duplicated logic, lib promotion) after feature work lands, the `refactor` agent (see `.opencode/agents/refactor.md`) runs its own loop: explore subagents write `notes-<topic>.md`, it scopes a bound refactor brief, delegates to a coder, and routes the reviewer verdict. It never edits source directly and never commits.

**Role boundaries:** `AGENTS.md` = policy; `.agents/skills/*` = conventions; `.opencode/agents/*` = orchestration + permissions.

**Artifacts:** everything lives in the issue dir (`.agents/temp/issue-N-short-description/`) — `plan.md`, `phase-<id>-<name>.md`, `notes-<topic>.md`, `report-<task-id>.md`, `review-<task-id>.md`.
