use crate::{
    error::{Error, Result, RunError},
    resource::{Res, Resources},
    scope::{TaskScope, run_scoped, run_scoped_direct},
    task::{IntoTask, StoredTask, TaskContext},
};
use std::{mem, time::Duration};
use tokio_util::sync::CancellationToken;

/// Runtime owner for resources, root tasks, and deferred cleanup.
///
/// `App` is built after setup. It can run one-off tasks, spawn root tasks,
/// request app-wide shutdown, join the current root task set, and run deferred
/// cleanup callbacks.
pub struct App {
    defers: Vec<StoredTask>,
    resources: Resources,
    config: AppConfig,
    root: TaskScope,
    shutdown: CancellationToken,
}

/// Builds an app by collecting setup-time resources, config, and plugins.
pub struct AppBuilder {
    defers: Vec<StoredTask>,
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

/// Adds setup-time behavior to an app builder.
pub trait Plugin {
    fn setup(&self, _builder: &mut AppBuilder) {}
}

impl<T> Plugin for &T
where
    T: Plugin + ?Sized,
{
    fn setup(&self, builder: &mut AppBuilder) {
        (*self).setup(builder);
    }
}

impl App {
    #[allow(clippy::new_without_default)]
    /// Creates an empty app.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Creates a builder for configuring an app before runtime.
    pub fn builder() -> AppBuilder {
        AppBuilder::new()
    }

    /// Sets how long parent failure waits for child cancellation before aborting children.
    ///
    /// The default is `Duration::ZERO`, which requests cancellation and then
    /// immediately aborts children that are still running.
    pub fn cancel_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.config.cancel_timeout = timeout;
        self
    }

    /// Spawns a root task into this app's root task scope.
    ///
    /// Successful task values are ignored. Root task failures propagate to
    /// `join` after retry policy is exhausted.
    pub fn spawn<I, T>(&mut self, task: T) -> &mut Self
    where
        T: IntoTask<I> + Send + Sync + 'static,
        T::Output: Send + 'static,
        T::Error: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
    {
        let context = TaskContext::root(
            self.resources.clone(),
            self.config.cancel_timeout,
            self.shutdown.clone(),
        )
        .with_scope(self.root.clone());

        self.root.spawn_direct(task, context);
        self
    }

    /// Waits for all currently spawned root tasks to complete.
    pub async fn join(&mut self) -> Result<()> {
        let root = self.root.clone();
        let cancel_timeout = self.config.cancel_timeout;
        let mut join = tokio::spawn(async move { root.join_propagating(cancel_timeout).await });

        let root_result = tokio::select! {
            result = &mut join => result.map_err(Error::source)?,
            _ = self.shutdown.cancelled() => {
                self.root.cancel_with(cancel_timeout).await;
                join.await.map_err(Error::source)?
            }
        };

        let defer_result = self.run_defers().await;
        self.reset_lifecycle();

        match (root_result, defer_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    /// Requests app-wide cancellation without waiting for tasks to finish.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        self.root.cancel();
    }

    fn reset_lifecycle(&mut self) {
        self.root = TaskScope::default();
        self.shutdown = CancellationToken::new();
    }

    async fn run_defers(&mut self) -> Result<()> {
        let defers = mem::take(&mut self.defers);
        let context = TaskContext::root(
            self.resources.clone(),
            self.config.cancel_timeout,
            self.shutdown.clone(),
        );
        let mut first_error = None;

        for defer in defers {
            if let Err(error) = run_scoped(defer, context.clone()).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Registers a deferred cleanup callback for the current root lifecycle.
    pub fn defer<I, T>(&mut self, task: T) -> &mut Self
    where
        T: IntoTask<I> + Send + Sync + 'static,
        T::Output: Send + 'static,
        T::Error: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
    {
        self.defers.push(StoredTask::from_task(task));
        self
    }

    /// Runs one task against the app's resources.
    ///
    /// This executes independently of the root task set.
    pub async fn run<I, T>(&self, task: T) -> std::result::Result<T::Output, RunError<T::Error>>
    where
        T: IntoTask<I>,
    {
        let context = TaskContext::root(
            self.resources.clone(),
            self.config.cancel_timeout,
            CancellationToken::new(),
        );
        run_scoped_direct(&task, context).await
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
}

impl AppBuilder {
    #[allow(clippy::new_without_default)]
    /// Creates an empty app builder.
    pub fn new() -> Self {
        Self {
            defers: Vec::new(),
            resources: Resources::default(),
            config: AppConfig::default(),
        }
    }

    /// Builds an app from the current builder state.
    ///
    /// The builder is drained, so calling `build` again creates an empty app.
    pub fn build(&mut self) -> App {
        App {
            defers: mem::take(&mut self.defers),
            resources: mem::take(&mut self.resources),
            config: mem::take(&mut self.config),
            root: TaskScope::default(),
            shutdown: CancellationToken::new(),
        }
    }

    /// Runs a plugin's setup against this builder.
    pub fn add_plugin<P>(&mut self, plugin: P) -> &mut Self
    where
        P: Plugin,
    {
        plugin.setup(self);
        self
    }

    /// Sets how long parent failure waits for child cancellation before aborting children.
    pub fn cancel_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.config.cancel_timeout = timeout;
        self
    }

    /// Inserts or replaces a resource in the app being built.
    pub fn override_resource<R: Send + Sync + 'static>(&mut self, res: R) -> &mut Self {
        self.resources.override_with(res);
        self
    }

    /// Inserts a resource into the app being built.
    ///
    /// Panics if a resource of the same type has already been inserted.
    pub fn insert_resource<R: Send + Sync + 'static>(&mut self, res: R) -> &mut Self {
        self.resources.insert(res);
        self
    }

    pub(crate) fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
    }
}

#[cfg(test)]
mod tests {
    use super::App;

    #[test]
    #[should_panic(expected = "i32 is already registered")]
    fn add_resource_panics_when_resource_type_is_registered_twice() {
        App::builder().insert_resource(5i32).insert_resource(9i32);
    }
}
