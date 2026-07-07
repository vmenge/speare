use crate::{
    error::Result,
    task::{TaskContext, TaskParam},
};
use tokio_util::sync::CancellationToken;

/// Allows a task to request app shutdown.
#[derive(Clone)]
pub struct Shutdown {
    token: CancellationToken,
}

impl Shutdown {
    /// Requests app shutdown.
    pub fn now(&self) {
        self.token.cancel();
    }
}

impl TaskParam for Shutdown {
    fn extract(context: &TaskContext) -> Result<Self> {
        Ok(Self {
            token: context.shutdown().clone(),
        })
    }
}
