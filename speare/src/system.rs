use crate::{
    error::{IntoSystemResult, Result},
    resource::Resources,
    retry::SystemPolicy,
    scope::TaskScope,
};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) type SystemFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

#[derive(Clone)]
pub(crate) struct StoredSystem {
    run: Arc<dyn Fn(&SystemContext) -> SystemFuture + Send + Sync>,
    policy: SystemPolicy,
}

impl StoredSystem {
    pub(crate) fn new(run: Arc<dyn Fn(&SystemContext) -> SystemFuture + Send + Sync>) -> Self {
        Self {
            run,
            policy: SystemPolicy::default(),
        }
    }

    pub(crate) fn run(&self, context: &SystemContext) -> SystemFuture {
        (self.run)(context)
    }

    pub(crate) fn policy(&self) -> SystemPolicy {
        self.policy
    }

    pub(crate) fn with_policy(mut self, policy: SystemPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[derive(Clone)]
pub struct SystemContext {
    resources: Resources,
    scope: TaskScope,
    cancel_timeout: Duration,
    shutdown: CancellationToken,
}

impl SystemContext {
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

pub trait SystemParam {
    fn extract(context: &SystemContext) -> Result<Self>
    where
        Self: Sized;
}

pub trait IntoSystem<Input> {
    fn into_system(self) -> StoredSystem;
}

macro_rules! impl_system {
    ($($params:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<Fun, Fut, $($params,)*> IntoSystem<($($params,)*)> for Fun
        where
            Fun: Fn($($params),*) -> Fut + Send + Sync + 'static,
            Fut: Future + Send + 'static,
            Fut::Output: IntoSystemResult,
            $($params: SystemParam + 'static,)*
        {
            fn into_system(self) -> StoredSystem {
                StoredSystem::new(Arc::new(move |context| {
                    $(
                    let $params = match $params::extract(context) {
                        Ok(param) => param,
                        Err(err) => {
                            let future: SystemFuture = Box::pin(async move { Err(err) });
                            return future;
                        }
                    };
                    )*

                    let future = self($($params),*);
                    Box::pin(async move { future.await.into_system_result() })
                }))
            }
        }
    };
}

impl_system!();
impl_system!(T1);
impl_system!(T1, T2);
impl_system!(T1, T2, T3);
impl_system!(T1, T2, T3, T4);
impl_system!(T1, T2, T3, T4, T5);
impl_system!(T1, T2, T3, T4, T5, T6);
impl_system!(T1, T2, T3, T4, T5, T6, T7);
impl_system!(T1, T2, T3, T4, T5, T6, T7, T8);
impl_system!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_system!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
impl_system!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
impl_system!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
