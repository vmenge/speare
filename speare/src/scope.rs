use crate::{
    error::{Error, Result, RunError},
    task::{IntoTask, StoredTask, TaskContext, TaskFuture, TaskParam},
};
use std::time::Duration;
use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    sync::{Notify, broadcast},
    task::{AbortHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

/// Scope for the currently running task.
///
/// `Scope` can spawn child tasks. Spawned children are scoped to the current
/// task invocation: the current task is not considered complete until all
/// children complete.
#[derive(Clone)]
pub struct Scope {
    context: TaskContext,
}

impl Scope {
    /// Returns whether this task has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.context.scope().is_cancelled()
    }

    /// Waits until this task is cancelled.
    pub async fn cancelled(&self) {
        self.context.scope().cancelled().await
    }

    /// Spawns a child task in the current task scope.
    ///
    /// The child runs concurrently. This method returns immediately, but the
    /// current task invocation will wait for the child before completing.
    ///
    /// If the current task fails, all children are aborted. Child failures
    /// can be observed with `Child::failed` or `Scope::next_failure`.
    pub fn spawn<I, T>(&self, task: T) -> Child
    where
        T: IntoTask<I> + Send + Sync + 'static,
        T::Output: Send + 'static,
        T::Error: fmt::Display + fmt::Debug + Send + Sync + 'static,
    {
        self.context
            .scope()
            .spawn(StoredTask::from_task(task), self.context.clone())
    }

    /// Runs one nested task inside the current task scope.
    ///
    /// This waits for the nested task and any children it spawns before
    /// returning. The nested task's output and error type are preserved.
    pub async fn run<I, T>(&self, task: T) -> std::result::Result<T::Output, RunError<T::Error>>
    where
        T: IntoTask<I>,
    {
        run_scoped_direct(&task, self.context.clone()).await
    }

    /// Waits for the next child failure in this scope.
    ///
    /// This observes failures without consuming the child. The scope still waits
    /// for all children before the current task is considered complete.
    pub async fn next_failure(&self) -> ChildFailure {
        self.context.scope().next_failure().await
    }

    /// Aborts all children currently spawned in this scope.
    pub fn abort_all(&self) {
        self.context.scope().abort();
    }

    /// Requests cooperative cancellation for all children currently spawned in this scope.
    pub fn cancel_all(&self) {
        self.context.scope().cancel();
    }

    /// Cancels all children, waits up to `timeout`, then aborts unfinished children.
    pub async fn cancel_all_with(&self, timeout: Duration) {
        self.context.scope().cancel_with(timeout).await;
    }
}

impl TaskParam for Scope {
    fn extract(context: &TaskContext) -> Result<Self> {
        Ok(Self {
            context: context.clone(),
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct TaskScope {
    inner: Arc<TaskScopeInner>,
}

struct TaskScopeInner {
    next_id: AtomicUsize,
    children: Mutex<Vec<Child>>,
    tasks: Mutex<Option<JoinSet<ChildFinished>>>,
    failures: broadcast::Sender<ChildFailure>,
    cancellation: CancellationToken,
}

impl Default for TaskScopeInner {
    fn default() -> Self {
        let (failures, _) = broadcast::channel(128);

        Self {
            next_id: AtomicUsize::new(0),
            children: Mutex::new(Vec::new()),
            tasks: Mutex::new(Some(JoinSet::new())),
            failures,
            cancellation: CancellationToken::new(),
        }
    }
}

/// Opaque identifier for a child task spawned from a `Scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChildId(usize);

/// Failure emitted by a child task.
#[derive(Debug, Clone)]
pub struct ChildFailure {
    /// The child that failed.
    pub child_id: ChildId,
    /// The final failure after retry policy was exhausted.
    pub error: Error,
}

/// Handle to a child task spawned from a `Scope`.
#[derive(Clone)]
pub struct Child {
    id: ChildId,
    abort: AbortHandle,
    cancellation: CancellationToken,
    state: Arc<ChildState>,
}

impl Child {
    /// Returns this child task's identifier.
    pub fn id(&self) -> ChildId {
        self.id
    }

    /// Waits for this child to fail.
    ///
    /// If the child completes successfully, this waits forever.
    pub async fn failed(&self) -> Error {
        loop {
            if let Some(result) = self.state.result()
                && let Err(error) = result
            {
                return error;
            }

            self.state.notify.notified().await;
        }
    }

    /// Aborts this child task.
    pub fn abort(&self) {
        self.abort.abort();
        self.state.set_result(Ok(()));
    }

    /// Requests cooperative cancellation for this child.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Cancels this child, waits up to `timeout`, then aborts it if still running.
    pub async fn cancel_with(&self, timeout: Duration) {
        self.cancel();

        if tokio::time::timeout(timeout, self.state.finished())
            .await
            .is_err()
        {
            self.abort();
            self.state.finished().await;
        }
    }
}

#[derive(Default)]
struct ChildState {
    result: Mutex<Option<Result<()>>>,
    notify: Notify,
}

impl ChildState {
    fn set_result(&self, result: Result<()>) {
        let mut current = self.result.lock().expect("child state is poisoned");

        if current.is_none() {
            *current = Some(result);
        }

        self.notify.notify_waiters();
    }

    fn result(&self) -> Option<Result<()>> {
        self.result.lock().expect("child state is poisoned").clone()
    }

    async fn finished(&self) {
        loop {
            if self
                .result
                .lock()
                .expect("child state is poisoned")
                .is_some()
            {
                return;
            }

            self.notify.notified().await;
        }
    }
}

struct ChildFinished {
    result: Result<()>,
    propagates_failure: bool,
}

impl TaskScope {
    fn child(&self) -> Self {
        let (failures, _) = broadcast::channel(128);
        let cancellation = self.inner.cancellation.child_token();

        if self.inner.cancellation.is_cancelled() {
            cancellation.cancel();
        }

        Self {
            inner: Arc::new(TaskScopeInner {
                next_id: AtomicUsize::new(0),
                children: Mutex::new(Vec::new()),
                tasks: Mutex::new(Some(JoinSet::new())),
                failures,
                cancellation,
            }),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.inner.cancellation.is_cancelled()
    }

    async fn cancelled(&self) {
        self.inner.cancellation.cancelled().await
    }

    pub(crate) fn spawn(&self, task: StoredTask, context: TaskContext) -> Child {
        self.spawn_with(task, context, false)
    }

    pub(crate) fn spawn_direct<I, T>(&self, task: T, context: TaskContext) -> Child
    where
        T: IntoTask<I> + Send + Sync + 'static,
        T::Output: Send + 'static,
        T::Error: fmt::Display + fmt::Debug + Send + Sync + 'static,
    {
        let id = ChildId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let child_scope = self.child();
        let state = Arc::new(ChildState::default());
        let child_state = state.clone();
        let failures = self.inner.failures.clone();
        let cancellation = child_scope.inner.cancellation.clone();
        let child_context = context.with_scope(child_scope);

        let abort = self
            .inner
            .tasks
            .lock()
            .expect("task scope is poisoned")
            .as_mut()
            .expect("task scope is already joining")
            .spawn(async move {
                let result = match run_scoped_direct(&task, child_context).await {
                    Ok(_) => Ok(()),
                    Err(RunError::App(error)) => Err(error),
                    Err(RunError::Task(error)) => Err(Error::report(error)),
                };

                if let Err(error) = &result {
                    let failure = ChildFailure {
                        child_id: id,
                        error: error.clone(),
                    };

                    let _ = failures.send(failure);
                }

                child_state.set_result(result.clone());
                ChildFinished {
                    result,
                    propagates_failure: true,
                }
            });

        let child = Child {
            id,
            abort,
            cancellation,
            state,
        };

        self.inner
            .children
            .lock()
            .expect("task scope is poisoned")
            .push(child.clone());

        child
    }

    fn spawn_with(
        &self,
        task: StoredTask,
        context: TaskContext,
        propagates_failure: bool,
    ) -> Child {
        let id = ChildId(self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let child_scope = self.child();
        let state = Arc::new(ChildState::default());
        let child_state = state.clone();
        let failures = self.inner.failures.clone();
        let cancellation = child_scope.inner.cancellation.clone();
        let child_context = context.with_scope(child_scope);

        let abort = self
            .inner
            .tasks
            .lock()
            .expect("task scope is poisoned")
            .as_mut()
            .expect("task scope is already joining")
            .spawn(async move {
                let result = run_scoped_raw(task, child_context).await;

                if let Err(error) = &result {
                    let failure = ChildFailure {
                        child_id: id,
                        error: error.clone(),
                    };

                    let _ = failures.send(failure);
                }

                child_state.set_result(result.clone());
                ChildFinished {
                    result,
                    propagates_failure,
                }
            });

        let child = Child {
            id,
            abort,
            cancellation,
            state,
        };

        self.inner
            .children
            .lock()
            .expect("task scope is poisoned")
            .push(child.clone());

        child
    }

    fn abort(&self) {
        let children = self.inner.children.lock().expect("task scope is poisoned");

        for child in children.iter() {
            child.abort();
        }
    }

    pub(crate) fn cancel(&self) {
        self.inner.cancellation.cancel();

        let children = self.inner.children.lock().expect("task scope is poisoned");

        for child in children.iter() {
            child.cancel();
        }
    }

    pub(crate) async fn cancel_with(&self, timeout: Duration) {
        self.inner.cancellation.cancel();

        let children = self
            .inner
            .children
            .lock()
            .expect("task scope is poisoned")
            .clone();

        for child in &children {
            child.cancel();
        }

        if tokio::time::timeout(timeout, async {
            for child in &children {
                child.state.finished().await;
            }
        })
        .await
        .is_err()
        {
            for child in &children {
                child.abort();
            }

            for child in &children {
                child.state.finished().await;
            }
        }
    }

    async fn next_failure(&self) -> ChildFailure {
        let mut failures = self.inner.failures.subscribe();

        loop {
            match failures.recv().await {
                Ok(failure) => return failure,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => std::future::pending().await,
            }
        }
    }

    pub(crate) async fn join(&self) -> Result<()> {
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .expect("task scope is poisoned")
            .take()
            .expect("task scope is already joining");

        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(ChildFinished {
                    result: Err(error),
                    propagates_failure: true,
                }) => {
                    tasks.abort_all();
                    return Err(error);
                }
                Ok(ChildFinished { .. }) => {}
                Err(error) if error.is_cancelled() => {}
                Err(error) => {
                    tasks.abort_all();
                    return Err(Error::source(error));
                }
            }
        }

        Ok(())
    }

    pub(crate) async fn join_propagating(&self, cancel_timeout: Duration) -> Result<()> {
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .expect("task scope is poisoned")
            .take()
            .expect("task scope is already joining");

        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(ChildFinished {
                    result: Err(error),
                    propagates_failure: true,
                }) => {
                    self.cancel_with(cancel_timeout).await;
                    tasks.abort_all();
                    return Err(error);
                }
                Ok(ChildFinished { .. }) => {}
                Err(error) if error.is_cancelled() => {}
                Err(error) => {
                    self.cancel_with(cancel_timeout).await;
                    tasks.abort_all();
                    return Err(Error::source(error));
                }
            }
        }

        Ok(())
    }
}

pub(crate) fn run_scoped(task: StoredTask, context: TaskContext) -> TaskFuture {
    run_scoped_raw(task, context)
}

pub(crate) async fn run_scoped_direct<I, T>(
    task: &T,
    context: TaskContext,
) -> std::result::Result<T::Output, RunError<T::Error>>
where
    T: IntoTask<I>,
{
    let policy = task.restart_policy();
    let mut retries = 0;

    loop {
        let scope = context.scope().child();
        let scoped_context = context.with_scope(scope.clone());
        let result = task
            .run_task(&scoped_context)
            .await
            .map_err(RunError::App)?;

        let result = match result {
            Ok(value) => scope.join().await.map_err(RunError::App).map(|()| value),
            Err(error) => {
                scope.cancel_with(context.cancel_timeout()).await;
                Err(RunError::Task(error))
            }
        };

        match result {
            Ok(value) => return Ok(value),
            Err(_error) if policy.should_retry(retries) => {
                retries += 1;

                let delay = policy.delay(retries);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn run_scoped_raw(task: StoredTask, context: TaskContext) -> TaskFuture {
    Box::pin(async move {
        let policy = task.policy();
        let mut retries = 0;

        loop {
            let scope = context.scope().child();
            let scoped_context = context.with_scope(scope.clone());
            let result = task.run(&scoped_context).await;

            let result = match result {
                Ok(()) => scope.join().await,
                Err(error) => {
                    scope.cancel_with(context.cancel_timeout()).await;
                    Err(error)
                }
            };

            match result {
                Ok(()) => return Ok(()),
                Err(_error) if policy.restart.should_retry(retries) => {
                    retries += 1;

                    let delay = policy.restart.delay(retries);
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                }
                Err(error) => return Err(error),
            }
        }
    })
}
