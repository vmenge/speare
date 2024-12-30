use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SyncVec<T> {
    x: Arc<Mutex<Vec<T>>>,
}

impl<T> Default for SyncVec<T> {
    fn default() -> Self {
        Self {
            x: Default::default(),
        }
    }
}

impl<T> Clone for SyncVec<T> {
    fn clone(&self) -> Self {
        Self { x: self.x.clone() }
    }
}

impl<T> SyncVec<T> {
    pub async fn push(&self, value: T) {
        self.x.lock().await.push(value)
    }
}

impl<T> SyncVec<T>
where
    T: Clone,
{
    pub async fn clone_vec(&self) -> Vec<T> {
        self.x.lock().await.clone()
    }
}
