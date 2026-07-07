use crate::{
    channel::{Broadcasts, Channels},
    error::{Error, Result},
    query::Queries,
    resource::{Res, Resources},
    scope::{TaskScope, run_scoped},
    system::{IntoSystem, StoredSystem, SystemContext},
};
use std::{collections::HashMap, time::Duration};
use tokio_util::sync::CancellationToken;

/// An application composed from resources and scheduled systems.
///
/// `App` is configured during startup. Calling `start`, `spawn`, or `run`
/// executes systems against a cloned snapshot of the app's current resources and
/// systems.
pub struct App {
    systems: HashMap<Schedule, Vec<StoredSystem>>,
    resources: Resources,
    config: AppConfig,
}

#[derive(Debug, Clone)]
struct AppConfig {
    cancel_timeout: Duration,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            cancel_timeout: Duration::ZERO,
        }
    }
}

/// A handle to an app running in a background Tokio task.
///
/// Use `join` to wait for the app to finish and observe its result, or `abort`
/// to cancel the background task.
pub struct AppHandle {
    task: tokio::task::JoinHandle<Result<()>>,
}

/// A named phase in the app lifecycle.
///
/// Systems are registered into schedules with `App::add_system`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Schedule {
    /// Runs once before task systems are spawned.
    ///
    /// Systems in this schedule run sequentially.
    Start,
    /// Runs after `Start` and before `Shutdown`.
    ///
    /// Systems in this schedule are spawned concurrently and awaited together.
    Work,
    /// Runs when the app is exiting.
    ///
    /// Shutdown systems run after normal completion, after shutdown is
    /// requested, and after startup or task errors. Every shutdown system is
    /// attempted before the app returns.
    Shutdown,
}

impl App {
    #[allow(clippy::new_without_default)]
    /// Creates an empty app.
    pub fn new() -> Self {
        Self {
            systems: HashMap::new(),
            resources: Resources::default(),
            config: AppConfig::default(),
        }
    }

    /// Sets how long parent failure waits for child cancellation before aborting children.
    ///
    /// The default is `Duration::ZERO`, which requests cancellation and then
    /// immediately aborts children that are still running.
    pub fn cancel_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.config.cancel_timeout = timeout;
        self
    }

    /// Starts the app in a background Tokio task.
    ///
    /// The spawned app uses a snapshot of the app's systems and resources at
    /// the time `spawn` is called. Changes made to the `App` after spawning do
    /// not affect the running task.
    pub fn spawn(&self) -> AppHandle {
        self.runtime().spawn()
    }

    /// Starts the app and waits for it to finish.
    ///
    /// This runs the `Start` schedule, then the `Work` schedule, then the
    /// `Shutdown` schedule. Like `spawn`, it executes a snapshot of the app's
    /// systems and resources.
    pub async fn start(&self) -> Result<()> {
        self.runtime().start().await
    }

    /// Runs one system against the app's resources.
    ///
    /// This does not add the system to a schedule and does not run any
    /// scheduled systems. It is useful for tests and one-off system execution.
    pub async fn run<I>(&self, system: impl IntoSystem<I>) -> Result<()> {
        let context = SystemContext::root(
            self.resources.clone(),
            self.config.cancel_timeout,
            CancellationToken::new(),
        );
        run_scoped(system.into_system(), context).await
    }

    fn runtime(&self) -> Runtime {
        Runtime {
            systems: self.systems.clone(),
            resources: self.resources.clone(),
            config: self.config.clone(),
        }
    }

    /// Adds a system to one of the app schedules.
    ///
    /// Systems in `Start` and `Shutdown` run sequentially. Systems in `Work`
    /// are spawned concurrently and awaited before shutdown begins.
    pub fn add_system<I>(&mut self, schedule: Schedule, system: impl IntoSystem<I>) -> &mut Self {
        self.systems
            .entry(schedule)
            .or_default()
            .push(system.into_system());
        self
    }

    /// Registers an unbounded channel for messages of type `T`.
    ///
    /// Systems can access this channel using `Sender<T>` and `Receiver<T>`.
    /// Receivers are competing consumers: each sent message is received by one
    /// receiver, not broadcast to every receiver.
    pub fn register_channel<T>(&mut self) -> &mut Self
    where
        T: Send + Sync + 'static,
    {
        self.resources
            .get_or_insert_with(Channels::default)
            .register::<T>(flume::unbounded());
        self
    }

    /// Registers a bounded channel for messages of type `T`.
    ///
    /// Sending waits for capacity when the channel is full. Try-sending returns
    /// immediately when the channel is full.
    ///
    /// Systems can access this channel using `Sender<T>` and `Receiver<T>`.
    /// Receivers are competing consumers: each sent message is received by one
    /// receiver, not broadcast to every receiver.
    pub fn register_bounded_channel<T>(&mut self, capacity: usize) -> &mut Self
    where
        T: Send + Sync + 'static,
    {
        self.resources
            .get_or_insert_with(Channels::default)
            .register::<T>(flume::bounded(capacity));
        self
    }

    /// Registers a broadcast for messages of type `T`.
    ///
    /// Systems can publish using `Pub<T>` and subscribe using `Sub<T>`.
    /// Each subscriber receives its own copy of each message published after it
    /// subscribes. Slow subscribers may lag and miss messages when more than
    /// `capacity` messages are published before they receive them.
    pub fn register_broadcast<T>(&mut self, capacity: usize) -> &mut Self
    where
        T: Clone + Send + Sync + 'static,
    {
        self.resources
            .get_or_insert_with(Broadcasts::default)
            .register::<T>(capacity);
        self
    }

    /// Registers an unbounded request/response query for `Req` requests and `Res` responses.
    ///
    /// Query workers can receive requests with `Query<Req, Res>`. Callers can
    /// send requests and await replies with `Querier<Req, Res>`.
    pub fn register_query<Req, Res>(&mut self) -> &mut Self
    where
        Req: Send + 'static,
        Res: Send + 'static,
    {
        self.resources
            .get_or_insert_with(Queries::default)
            .register::<Req, Res>(flume::unbounded());
        self
    }

    /// Registers a bounded request/response query for `Req` requests and `Res` responses.
    ///
    /// `Querier::query` waits for capacity when the queue is full.
    /// `Querier::try_query` returns immediately if the queue is full.
    pub fn register_bounded_query<Req, Res>(&mut self, capacity: usize) -> &mut Self
    where
        Req: Send + 'static,
        Res: Send + 'static,
    {
        self.resources
            .get_or_insert_with(Queries::default)
            .register::<Req, Res>(flume::bounded(capacity));
        self
    }

    /// Gets a resource from the app.
    ///
    /// Returns an error if the resource has not been inserted.
    pub fn get<R>(&self) -> Result<Res<R>>
    where
        R: Send + Sync + 'static,
    {
        self.resources.get::<R>().map(Res::new)
    }

    /// Inserts or replaces a resource in the app.
    ///
    /// This is intended for tests and configuration overrides. Unlike
    /// `insert_resource`, this does not panic when a resource of the same type
    /// already exists.
    ///
    /// Panics if resources have already been shared with a running system and
    /// the resource store can no longer be mutated.
    pub fn override_resource<R: Send + Sync + 'static>(&mut self, res: R) -> &mut Self {
        self.resources.override_with(res);
        self
    }

    /// Inserts a resource into the app.
    ///
    /// Panics if a resource of the same type has already been inserted.
    pub fn insert_resource<R: Send + Sync + 'static>(&mut self, res: R) -> &mut Self {
        self.resources.insert(res);
        self
    }
}

impl AppHandle {
    /// Waits for the app task to finish.
    ///
    /// Returns the app result, or a task join error if the background task
    /// panicked or was cancelled.
    pub async fn join(self) -> Result<()> {
        self.task.await.map_err(Error::source)?
    }

    /// Cancels the background app task.
    pub fn abort(&self) {
        self.task.abort();
    }
}

struct Runtime {
    systems: HashMap<Schedule, Vec<StoredSystem>>,
    resources: Resources,
    config: AppConfig,
}

impl Runtime {
    fn spawn(self) -> AppHandle {
        AppHandle {
            task: tokio::spawn(async move { self.start().await }),
        }
    }

    async fn start(self) -> Result<()> {
        let shutdown = CancellationToken::new();

        let primary_error = match self.run_start(shutdown.clone()).await {
            Ok(()) if shutdown.is_cancelled() => None,
            Ok(()) => self.run_tasks(shutdown.clone()).await.err(),
            Err(error) => Some(error),
        };

        let shutdown_error = self.run_shutdown(shutdown).await.err();

        match (primary_error, shutdown_error) {
            (Some(error), _) => Err(error),
            (None, Some(error)) => Err(error),
            (None, None) => Ok(()),
        }
    }

    async fn run_start(&self, shutdown: CancellationToken) -> Result<()> {
        if let Some(systems) = self.systems.get(&Schedule::Start) {
            let context = SystemContext::root(
                self.resources.clone(),
                self.config.cancel_timeout,
                shutdown.clone(),
            );

            for system in systems {
                run_scoped(system.clone(), context.clone()).await?;

                if shutdown.is_cancelled() {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn run_tasks(&self, shutdown: CancellationToken) -> Result<()> {
        if let Some(systems) = self.systems.get(&Schedule::Work) {
            let root = TaskScope::default();
            let context = SystemContext::root(
                self.resources.clone(),
                self.config.cancel_timeout,
                shutdown.clone(),
            )
            .with_scope(root.clone());

            for system in systems {
                root.spawn_propagating(system.clone(), context.clone());
            }

            let join_scope = root.clone();
            let mut join = tokio::spawn(async move { join_scope.join().await });

            tokio::select! {
                result = &mut join => result.map_err(Error::source)??,
                _ = shutdown.cancelled() => {
                    root.cancel_with(self.config.cancel_timeout).await;
                    join.await.map_err(Error::source)??;
                }
            }
        }

        Ok(())
    }

    async fn run_shutdown(&self, shutdown: CancellationToken) -> Result<()> {
        let mut first_error = None;

        if let Some(systems) = self.systems.get(&Schedule::Shutdown) {
            let context = SystemContext::root(
                self.resources.clone(),
                self.config.cancel_timeout,
                shutdown,
            );

            for system in systems {
                if let Err(error) = run_scoped(system.clone(), context.clone()).await
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::App;

    #[test]
    #[should_panic(expected = "i32 is already registered")]
    fn add_resource_panics_when_resource_type_is_registered_twice() {
        App::new().insert_resource(5i32).insert_resource(9i32);
    }
}
