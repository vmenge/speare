use crate::system::{IntoSystem, StoredSystem};
use std::time::Duration;

/// Controls whether a failed system should be restarted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Do not restart the system after a failure.
    #[default]
    Never,
    /// Restart the system forever until it succeeds or the running task is cancelled.
    Always,
    /// Restart the system at most `usize` times after the initial attempt.
    Max(usize),
}

/// Controls how long to wait before restarting a failed system.
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

/// Restart behavior for a system.
///
/// This is the lower-level policy behind `SystemExt::retry`. It is intentionally
/// scoped to one system; broader supervision behavior belongs around task scopes.
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
pub(crate) struct SystemPolicy {
    pub(crate) restart: RestartPolicy,
}

impl SystemPolicy {
    pub(crate) fn with_retry(mut self, retry: Retry) -> Self {
        self.restart = RestartPolicy::new(retry);
        self
    }

    pub(crate) fn with_backoff(mut self, backoff: Backoff) -> Self {
        self.restart = self.restart.with_backoff(backoff);
        self
    }
}

/// A system wrapped with runtime policy.
pub struct PolicySystem<S> {
    system: S,
    policy: SystemPolicy,
}

/// A system wrapped with restart behavior.
pub type RetrySystem<S> = PolicySystem<S>;

impl<S> PolicySystem<S> {
    /// Restarts the system according to `retry` when it returns an error.
    pub fn retry(mut self, retry: Retry) -> Self {
        self.policy = self.policy.with_retry(retry);
        self
    }

    /// Sets the backoff used before restarting the system.
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.policy = self.policy.with_backoff(backoff);
        self
    }
}

/// Extension methods for systems.
pub trait SystemExt: Sized {
    /// Restarts the system according to `retry` when it returns an error.
    fn retry(self, retry: Retry) -> PolicySystem<Self> {
        PolicySystem {
            system: self,
            policy: SystemPolicy::default().with_retry(retry),
        }
    }
}

impl<S> SystemExt for S {}

impl<Input, S> IntoSystem<Input> for PolicySystem<S>
where
    S: IntoSystem<Input>,
{
    fn into_system(self) -> StoredSystem {
        self.system.into_system().with_policy(self.policy)
    }
}
