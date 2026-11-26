use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use tokio::task::{Id, JoinError, JoinSet};

use crate::deadline::within_deadline;
use crate::exit_status::ExitStatus;
use crate::lifecycle::RunOutcome;
use crate::run_error::RunError;
use crate::shutdown::Shutdown;

/// Name logged for a task whose name was not recorded.
const SUPERVISOR_UNNAMED_TASK: &str = "unnamed";

/// How a supervised run ended, and which task caused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Supervised {
    /// The run outcome.
    pub outcome: RunOutcome,
    /// The first task that stopped with a non-success status, if any.
    pub culprit: Option<&'static str>,
}

/// Runs named critical tasks that share one [`Shutdown`]. The first task to
/// stop with an error, a panic, or an `Ok` before shutdown was requested
/// triggers the shutdown for all the others and becomes the culprit. Once the
/// shutdown is triggered, the remaining tasks have the deadline (see
/// [`Supervisor::run`]) to stop before they are aborted.
#[derive(Debug)]
pub struct Supervisor {
    shutdown: Shutdown,
    tasks: JoinSet<Result<(), RunError>>,
    names: HashMap<Id, &'static str>,
}

impl Supervisor {
    /// A supervisor with no tasks.
    #[must_use]
    pub fn new(shutdown: Shutdown) -> Self {
        Self {
            shutdown,
            tasks: JoinSet::new(),
            names: HashMap::new(),
        }
    }

    /// Spawns a named task into the supervisor.
    ///
    /// # Panics
    ///
    /// Panics when called outside a Tokio runtime.
    pub fn spawn<F>(&mut self, name: &'static str, task: F)
    where
        F: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        let handle = self.tasks.spawn(task);
        self.names.insert(handle.id(), name);
    }

    /// Runs every spawned task to completion, unless the shutdown deadline
    /// passes first.
    ///
    /// The deadline clock starts only once the [`Shutdown`] is triggered
    /// (immediately if it already was), whether by an OS signal or by a task
    /// stopping. The first task to stop with a non-success status, to panic,
    /// or to return an `Ok` before shutdown was requested triggers the
    /// shutdown for all the others and becomes the culprit. Once the shutdown
    /// is triggered, the remaining tasks have `deadline` to stop before they
    /// are aborted; aborted tasks are never awaited, because a task stuck in
    /// sync code would hang the run.
    ///
    /// The reported status is the first culprit's [`ExitStatus`]; otherwise
    /// [`ExitStatus::Interrupted`] when the deadline passes; otherwise
    /// [`ExitStatus::Success`]. With no tasks, returns
    /// [`ExitStatus::Success`] immediately.
    pub async fn run(self, deadline: Duration) -> Supervised {
        let Self {
            shutdown,
            mut tasks,
            mut names,
        } = self;
        let mut culprit: Option<(&'static str, ExitStatus)> = None;
        let drained = within_deadline(&shutdown, deadline, async {
            while let Some(joined) = tasks.join_next_with_id().await {
                let (id, result) = match joined {
                    Ok((id, result)) => (id, Ok(result)),
                    Err(error) => (error.id(), Err(error)),
                };
                let name = names.remove(&id).unwrap_or(SUPERVISOR_UNNAMED_TASK);
                let status = classify(name, shutdown.is_triggered(), result);
                shutdown.trigger();
                if status != ExitStatus::Success && culprit.is_none() {
                    culprit = Some((name, status));
                }
            }
        })
        .await;
        let deadline_exceeded = drained.is_err();
        if deadline_exceeded {
            let abandoned: Vec<&'static str> = names.values().copied().collect();
            tracing::warn!(
                tasks = ?abandoned,
                deadline_secs = deadline.as_secs(),
                "shutdown deadline exceeded, aborting tasks"
            );
            tasks.abort_all();
        }
        let status = match (culprit, deadline_exceeded) {
            (Some((_, status)), _) => status,
            (None, true) => ExitStatus::Interrupted,
            (None, false) => ExitStatus::Success,
        };
        Supervised {
            outcome: RunOutcome {
                status,
                deadline_exceeded,
            },
            culprit: culprit.map(|(name, _)| name),
        }
    }
}

fn classify(
    name: &'static str,
    was_triggered: bool,
    result: Result<Result<(), RunError>, JoinError>,
) -> ExitStatus {
    match result {
        Ok(Ok(())) if was_triggered => ExitStatus::Success,
        Ok(Ok(())) => {
            tracing::error!(task = name, "task stopped before shutdown was requested");
            ExitStatus::Retryable
        }
        Ok(Err(error)) => {
            tracing::error!(task = name, %error, "task failed");
            error.exit_status()
        }
        Err(error) if error.is_panic() => {
            tracing::error!(task = name, "task panicked");
            ExitStatus::Retryable
        }
        Err(_) => {
            tracing::warn!(task = name, "task cancelled");
            ExitStatus::Interrupted
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::time::Duration;

    use tokio::sync::oneshot;
    use tokio::time::{Instant, timeout};

    use super::{Supervised, Supervisor};
    use crate::exit_status::ExitStatus;
    use crate::lifecycle::RunOutcome;
    use crate::run_error::RunError;
    use crate::shutdown::Shutdown;

    fn stops_on_shutdown(
        shutdown: &Shutdown,
    ) -> impl Future<Output = Result<(), RunError>> + Send + 'static {
        let shutdown = shutdown.clone();
        async move {
            shutdown.triggered().await;
            Ok(())
        }
    }

    async fn ignores_shutdown() -> Result<(), RunError> {
        std::future::pending().await
    }

    #[tokio::test(start_paused = true)]
    async fn no_tasks_is_success() {
        let supervised = timeout(
            Duration::from_secs(60),
            Supervisor::new(Shutdown::new()).run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Success),
                culprit: None,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tasks_stopping_on_external_shutdown_is_success() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("one", stops_on_shutdown(&shutdown));
        supervisor.spawn("two", stops_on_shutdown(&shutdown));

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Success),
                culprit: None,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn task_returning_before_shutdown_is_retryable() {
        let shutdown = Shutdown::new();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("early", async { Ok::<(), RunError>(()) });
        supervisor.spawn("steady", stops_on_shutdown(&shutdown));

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some("early"),
            }
        );
        assert!(shutdown.is_triggered());
    }

    #[tokio::test(start_paused = true)]
    async fn task_error_maps_through_run_error() {
        let shutdown = Shutdown::new();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("failing", async { Err(RunError::permanent("bad config")) });
        supervisor.spawn("steady", stops_on_shutdown(&shutdown));

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Permanent),
                culprit: Some("failing"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn task_panic_is_retryable() {
        let shutdown = Shutdown::new();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("exploding", async { panic!("boom") });
        supervisor.spawn("steady", stops_on_shutdown(&shutdown));

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Retryable),
                culprit: Some("exploding"),
            }
        );
        assert!(!supervised.outcome.deadline_exceeded);
    }

    #[tokio::test(start_paused = true)]
    async fn first_failure_wins() {
        let shutdown = Shutdown::new();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("first", async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Err(RunError::permanent("first failure"))
        });
        let waiter = shutdown.clone();
        supervisor.spawn("second", async move {
            waiter.triggered().await;
            Err(RunError::retryable("second failure"))
        });

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::completed(ExitStatus::Permanent),
                culprit: Some("first"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_exceeded_without_culprit_is_interrupted() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let started = Instant::now();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("stuck", ignores_shutdown());

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::abandoned(ExitStatus::Interrupted),
                culprit: None,
            }
        );
        assert_eq!(started.elapsed(), Duration::from_secs(5));
    }

    #[tokio::test(start_paused = true)]
    async fn deadline_counts_from_the_trigger() {
        let shutdown = Shutdown::new();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("stuck", ignores_shutdown());
        let trigger = shutdown.clone();
        supervisor.spawn("trigger", async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            trigger.trigger();
            Ok(())
        });
        let started = Instant::now();

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::abandoned(ExitStatus::Interrupted),
                culprit: None,
            }
        );
        assert_eq!(started.elapsed(), Duration::from_secs(15));
    }

    #[tokio::test(start_paused = true)]
    async fn culprit_status_survives_the_deadline() {
        let shutdown = Shutdown::new();
        let mut supervisor = Supervisor::new(shutdown.clone());
        supervisor.spawn("failing", async { Err(RunError::permanent("bad config")) });
        supervisor.spawn("stuck", ignores_shutdown());

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(5)),
        )
        .await
        .expect("run completes");

        assert_eq!(
            supervised,
            Supervised {
                outcome: RunOutcome::abandoned(ExitStatus::Permanent),
                culprit: Some("failing"),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn abandoned_tasks_are_aborted() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        let mut supervisor = Supervisor::new(shutdown.clone());
        let (sender, receiver) = oneshot::channel::<()>();
        supervisor.spawn("stuck", async move {
            let _guard = sender;
            std::future::pending().await
        });

        let supervised = timeout(
            Duration::from_secs(60),
            supervisor.run(Duration::from_secs(1)),
        )
        .await
        .expect("run completes");

        assert!(supervised.outcome.deadline_exceeded);
        assert!(matches!(
            timeout(Duration::from_secs(1), receiver).await,
            Ok(Err(_))
        ));
    }
}
