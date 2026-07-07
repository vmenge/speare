use crate::{
    error::Result,
    system::{SystemContext, SystemParam},
};
use tokio_util::sync::CancellationToken;

/// Allows a system to request app shutdown.
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

impl SystemParam for Shutdown {
    fn extract(context: &SystemContext) -> Result<Self> {
        Ok(Self {
            token: context.shutdown().clone(),
        })
    }
}
