# runtime

Process lifecycle primitives shared by every deployable shape (API, worker,
job): the cancellation-token shutdown handle with OS-signal installation, the
shutdown deadline, the task supervisor, the bounded telemetry flush, the
lifecycle configuration, the Kubernetes-oriented exit-status taxonomy, the
retryable/permanent run-error classification, and the liveness heartbeat for
long-running loops. The crate is tokio-only: it never depends on axum, web or
telemetry, and `clap` is used only to parse [`LifecycleConfig`](src/config.rs).
Consumers use module paths (`runtime::shutdown::Shutdown`) because `lib.rs`
has no root re-exports.

## Contents

- [`Shutdown`](src/shutdown.rs) — cloneable, cancellation-token shutdown
  handle; `listen_for_signals` triggers it on `SIGTERM` (unix) or `Ctrl-C`,
  whichever arrives first, and a second shutdown signal ends the process with
  exit code 143 immediately, skipping any drain, deadline, or flush.
- [`ExitStatus`](src/exit_status.rs) — how a run ended, mapped to exit codes
  that Job `backoffLimit` and `podFailurePolicy` rules can match (see
  [Exit codes](#exit-codes)).
- [`RunError`](src/run_error.rs) — retryable/permanent run-failure
  classification; `exit_status` maps each variant to its [`ExitStatus`](#exit-codes)
  status.
- [`Heartbeat`](src/heartbeat.rs) — liveness signal for long-running loops;
  the loop calls `beat` at least every `interval`, `check` fails once no
  beat has landed within `stale_after`, and `beat_while` keeps it fresh
  while awaiting a future that can outlast `stale_after` (handlers,
  deliveries).
- [`LifecycleConfig`](src/config.rs) — CLI/env lifecycle configuration shared
  by every process shape: the drain delay, the task shutdown timeout, and the
  flush timeout, with accessors for the deadline (drain + timeout) and the
  termination budget (deadline + flush).
- [`within_deadline`](src/deadline.rs) — runs a future to completion unless
  `shutdown` is triggered and `deadline` then passes first; the deadline clock
  starts only at the trigger, and the losing future is dropped.
  [`DeadlineExceeded`](src/deadline.rs) names the deadline that passed.
- [`RunOutcome` / `flush_within` / `finish`](src/lifecycle.rs) — how a run
  ended (its [`ExitStatus`](src/exit_status.rs) plus whether work was
  abandoned at the deadline), [`FlushOutcome`](src/lifecycle.rs) and
  [`flush_within`](src/lifecycle.rs) — the blocking flush bounded by a
  timeout, whose failures are logged warnings — and [`finish`](src/lifecycle.rs),
  which flushes telemetry and returns the process exit code, calling
  `std::process::exit` instead of returning when work was abandoned or the
  flush timed out ([`must_exit_immediately`](src/lifecycle.rs)).
- [`Supervisor`](src/supervisor.rs) — runs named critical tasks that share
  one `Shutdown`; the first task to stop with an error, a panic, or an `Ok`
  before shutdown was requested triggers the shutdown for the others and
  becomes the culprit. [`Supervised`](src/supervisor.rs) reports the
  [`RunOutcome`](src/lifecycle.rs) and culprit; remaining tasks are aborted
  once the shutdown deadline passes.

## Configuration

| Env var | Default | Meaning |
|---|---|---|
| `SHUTDOWN_DRAIN_SECONDS` | `0` | Seconds to keep serving after the shutdown signal, with readiness reporting shutting down, before servers stop accepting connections. |
| `SHUTDOWN_TIMEOUT_SECONDS` | `20` | Seconds after the drain that tasks may take to stop before they are abandoned and the process exits 143; must be at least 1. |
| `SHUTDOWN_FLUSH_TIMEOUT_SECONDS` | `5` | Seconds the final telemetry flush may take; must be at least 1. |

CLI flags (`--shutdown-drain-seconds`, `--shutdown-timeout-seconds`,
`--shutdown-flush-timeout-seconds`) override env vars, which override these
defaults.

## Shutdown budget

The whole shutdown must fit inside the pod's `terminationGracePeriodSeconds`:

`SHUTDOWN_DRAIN_SECONDS + SHUTDOWN_TIMEOUT_SECONDS + SHUTDOWN_FLUSH_TIMEOUT_SECONDS < terminationGracePeriodSeconds`

The defaults give 0 + 20 + 5 = 25 < 30. The deadline
(`SHUTDOWN_DRAIN_SECONDS + SHUTDOWN_TIMEOUT_SECONDS`) starts at the moment
shutdown is triggered — by an OS signal or by a failing task — not at process
start. Work a task does during shutdown (for example the greetings producer's
`KAFKA_PRODUCER_FLUSH_TIMEOUT_SECONDS`, default `10`) runs inside the deadline
and must fit inside `SHUTDOWN_TIMEOUT_SECONDS`. When the deadline passes, the
remaining tasks are aborted, telemetry is flushed within
`SHUTDOWN_FLUSH_TIMEOUT_SECONDS`, and the process exits 143 — or with the
culprit's status if a task failed first. A second `SIGTERM`/`SIGINT` exits 143
immediately, skipping the drain, the deadline, and the flush.

## Exit codes

| Code | [`ExitStatus`](src/exit_status.rs) | Meaning |
|---|---|---|
| 0 | `Success` | The run completed, even if a shutdown signal arrived. |
| 1 | `Retryable` | A transient failure; counts toward the Job's `backoffLimit`. |
| 2 | `Permanent` | A failure retrying cannot fix (bad input, bad configuration). Match it with a `podFailurePolicy` `FailJob` rule so the Job fails without further restarts. |
| 143 | `Interrupted` | The run was cut short by a shutdown signal (128 + SIGTERM), or the shutdown deadline passed, or a second signal arrived. |
