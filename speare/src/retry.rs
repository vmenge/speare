use crate::task::IntoTask;
use std::time::Duration;

/// Controls whether a failed task should be restarted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Do not restart the task after a failure.
    #[default]
    Never,
    /// Restart the task forever until it succeeds or the running task is cancelled.
    Always,
    /// Restart the task at most `usize` times after the initial attempt.
    Max(usize),
}

/// Controls how long to wait before restarting a failed task.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Backoff {
    /// Restart immediately with no delay.
    #[default]
    None,
    /// Wait a fixed duration between restarts.
    Static(Duration),
    /// Increase delay from `min` to `max` by `step` per restart.
    Incremental {
        min: Duration,
        max: Duration,
        step: Duration,
    },
}

/// Restart behavior for a task.
///
/// This is the lower-level policy behind `TaskExt::retry`. It is intentionally
/// scoped to one task; broader supervision behavior belongs around task scopes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    retry: Retry,
    backoff: Backoff,
}

impl RestartPolicy {
    /// Creates a restart policy with no backoff.
    pub fn new(retry: Retry) -> Self {
        Self {
            retry,
            backoff: Backoff::default(),
        }
    }

    /// Sets the backoff used before each restart.
    pub fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    pub(crate) fn should_retry(self, retries: usize) -> bool {
        match self.retry {
            Retry::Never => false,
            Retry::Always => true,
            Retry::Max(max) => retries < max,
        }
    }

    pub(crate) fn delay(self, retries: usize) -> Duration {
        match self.backoff {
            Backoff::None => Duration::ZERO,
            Backoff::Static(duration) => duration,
            Backoff::Incremental { min, max, step } => {
                let Some(steps) = retries.checked_sub(1) else {
                    return min.min(max);
                };

                min.saturating_add(step.saturating_mul(steps as u32))
                    .min(max)
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TaskPolicy {
    pub(crate) restart: RestartPolicy,
}

impl TaskPolicy {
    pub(crate) fn with_retry(mut self, retry: Retry) -> Self {
        self.restart = RestartPolicy::new(retry);
        self
    }

    pub(crate) fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.restart = self.restart.with_backoff(backoff);
        self
    }
}

/// A task wrapped with runtime policy.
pub struct PolicyTask<S> {
    task: S,
    policy: TaskPolicy,
}

/// A task wrapped with restart behavior.
pub type RetryTask<S> = PolicyTask<S>;

impl<S> PolicyTask<S> {
    /// Restarts the task according to `retry` when it returns an error.
    pub fn retry(mut self, retry: Retry) -> Self {
        self.policy = self.policy.with_retry(retry);
        self
    }

    /// Sets the backoff used before restarting the task.
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.policy = self.policy.with_backoff(backoff);
        self
    }
}

/// Extension methods for tasks.
pub trait TaskExt: Sized {
    /// Restarts the task according to `retry` when it returns an error.
    fn retry(self, retry: Retry) -> PolicyTask<Self> {
        PolicyTask {
            task: self,
            policy: TaskPolicy::default().with_retry(retry),
        }
    }
}

impl<S> TaskExt for S {}

impl<Input, S> IntoTask<Input> for PolicyTask<S>
where
    S: IntoTask<Input>,
{
    type Output = S::Output;
    type Error = S::Error;

    fn run_task<'a>(
        &'a self,
        context: &'a crate::task::TaskContext,
    ) -> impl std::future::Future<
        Output = crate::Result<std::result::Result<Self::Output, Self::Error>>,
    > + Send
    + 'a {
        self.task.run_task(context)
    }

    fn restart_policy(&self) -> RestartPolicy {
        self.policy.restart
    }
}
