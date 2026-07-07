use crate::{
    error::{IntoTaskResult, Result},
    resource::Resources,
    retry::{RestartPolicy, TaskPolicy},
    scope::TaskScope,
};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) type TaskFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

#[derive(Clone)]
pub(crate) struct StoredTask {
    run: Arc<dyn Fn(&TaskContext) -> TaskFuture + Send + Sync>,
    policy: TaskPolicy,
}

impl StoredTask {
    pub(crate) fn new(run: Arc<dyn Fn(&TaskContext) -> TaskFuture + Send + Sync>) -> Self {
        Self {
            run,
            policy: TaskPolicy::default(),
        }
    }

    pub(crate) fn run(&self, context: &TaskContext) -> TaskFuture {
        (self.run)(context)
    }

    pub(crate) fn policy(&self) -> TaskPolicy {
        self.policy
    }

    pub(crate) fn with_policy(mut self, policy: TaskPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[derive(Clone)]
pub struct TaskContext {
    resources: Resources,
    scope: TaskScope,
    cancel_timeout: Duration,
    shutdown: CancellationToken,
}

impl TaskContext {
    pub(crate) fn root(
        resources: Resources,
        cancel_timeout: Duration,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            resources,
            scope: TaskScope::default(),
            cancel_timeout,
            shutdown,
        }
    }

    pub(crate) fn with_scope(&self, scope: TaskScope) -> Self {
        Self {
            resources: self.resources.clone(),
            scope,
            cancel_timeout: self.cancel_timeout,
            shutdown: self.shutdown.clone(),
        }
    }

    pub(crate) fn resources(&self) -> &Resources {
        &self.resources
    }

    pub(crate) fn scope(&self) -> &TaskScope {
        &self.scope
    }

    pub(crate) fn cancel_timeout(&self) -> Duration {
        self.cancel_timeout
    }

    pub(crate) fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }
}

pub trait TaskParam {
    fn extract(context: &TaskContext) -> Result<Self>
    where
        Self: Sized;
}

pub trait IntoTask<Input> {
    type Output;

    fn run_task<'a>(
        &'a self,
        context: &'a TaskContext,
    ) -> impl Future<Output = Result<Self::Output>> + Send + 'a;

    fn restart_policy(&self) -> RestartPolicy {
        RestartPolicy::default()
    }

    fn into_task(self) -> StoredTask;
}

macro_rules! impl_task {
    ($($params:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<Fun, Fut, $($params,)*> IntoTask<($($params,)*)> for Fun
        where
            Fun: Fn($($params),*) -> Fut + Send + Sync + 'static,
            Fut: Future + Send + 'static,
            Fut::Output: IntoTaskResult,
            $($params: TaskParam + 'static,)*
        {
            type Output = <Fut::Output as IntoTaskResult>::Output;

            fn run_task<'a>(
                &'a self,
                context: &'a TaskContext,
            ) -> impl Future<Output = Result<Self::Output>> + Send + 'a {
                async move {
                    $(
                    let $params = $params::extract(context)?;
                    )*

                    self($($params),*).await.into_task_result()
                }
            }

            fn into_task(self) -> StoredTask {
                StoredTask::new(Arc::new(move |context| {
                    $(
                    let $params = match $params::extract(context) {
                        Ok(param) => param,
                        Err(err) => {
                            let future: TaskFuture = Box::pin(async move { Err(err) });
                            return future;
                        }
                    };
                    )*

                    let future = self($($params),*);
                    Box::pin(async move { future.await.into_task_result().map(|_| ()) })
                }))
            }
        }
    };
}

impl_task!();
impl_task!(T1);
impl_task!(T1, T2);
impl_task!(T1, T2, T3);
impl_task!(T1, T2, T3, T4);
impl_task!(T1, T2, T3, T4, T5);
impl_task!(T1, T2, T3, T4, T5, T6);
impl_task!(T1, T2, T3, T4, T5, T6, T7);
impl_task!(T1, T2, T3, T4, T5, T6, T7, T8);
impl_task!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_task!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_task!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
impl_task!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
