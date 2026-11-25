# runtime

Process lifecycle primitives shared by every deployable shape (API, worker,
job): the cancellation-token shutdown handle with OS-signal installation, the
Kubernetes-oriented exit-status taxonomy, the retryable/permanent run-error
classification, and the liveness heartbeat for long-running loops. The crate is
tokio-only — it never depends on axum, web or telemetry — and consumers use
module paths (`runtime::shutdown::Shutdown`) because `lib.rs` has no root
re-exports.

## Contents

- [`Shutdown`](src/shutdown.rs) — cloneable, cancellation-token shutdown
  handle; `listen_for_signals` triggers it on `SIGTERM` (unix) or `Ctrl-C`,
  whichever arrives first.
- [`ExitStatus`](src/exit_status.rs) — how a run ended, mapped to exit codes
  that Job `backoffLimit` and `podFailurePolicy` rules can match (see
  [Exit codes](#exit-codes)).
- [`RunError`](src/run_error.rs) — retryable/permanent run-failure
  classification; `exit_status` maps each variant to its [`ExitStatus`](#exit-codes)
  status.
- [`Heartbeat`](src/heartbeat.rs) — liveness signal for long-running loops;
  the loop calls `beat` at least every `interval`, and `check` fails once no
  beat has landed within `stale_after`.

## Exit codes

| Code | [`ExitStatus`](src/exit_status.rs) | Meaning |
|---|---|---|
| 0 | `Success` | The run completed, even if a shutdown signal arrived. |
| 1 | `Retryable` | A transient failure; counts toward the Job's `backoffLimit`. |
| 2 | `Permanent` | A failure retrying cannot fix (bad input, bad configuration). Match it with a `podFailurePolicy` `FailJob` rule so the Job fails without further restarts. |
| 143 | `Interrupted` | The run was cut short by a shutdown signal (128 + SIGTERM). |
