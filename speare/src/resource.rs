use crate::{
    error::{Error, Result},
    task::{TaskContext, TaskParam},
};
use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
    ops::Deref,
    sync::Arc,
};

#[derive(Clone, Default)]
pub struct Resources(Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>);

impl Resources {
    pub(crate) fn insert<R>(&mut self, res: R)
    where
        R: Send + Sync + 'static,
    {
        let resources = Arc::get_mut(&mut self.0).expect("resources are already shared");

        if resources.insert(TypeId::of::<R>(), Arc::new(res)).is_some() {
            panic!(
                "{} is already registered as a resource. Cannot register the same resource type twice.",
                type_name::<R>()
            );
        }
    }

    pub(crate) fn get<R>(&self) -> Result<Arc<R>>
    where
        R: Send + Sync + 'static,
    {
        self.0
            .get(&TypeId::of::<R>())
            .ok_or(Error::MissingResource {
                type_name: type_name::<R>(),
            })?
            .clone()
            .downcast::<R>()
            .map_err(|_| Error::ResourceTypeMismatch {
                type_name: type_name::<R>(),
            })
    }

    pub(crate) fn override_with<R>(&mut self, res: R)
    where
        R: Send + Sync + 'static,
    {
        Arc::make_mut(&mut self.0).insert(TypeId::of::<R>(), Arc::new(res));
    }

    /// Gets mutable access to a resource, inserting it first if it is missing.
    ///
    /// Panics if resources have already been shared with running tasks.
    pub fn get_or_insert_with<R>(&mut self, f: impl FnOnce() -> R) -> &mut R
    where
        R: Send + Sync + 'static,
    {
        let resources = Arc::get_mut(&mut self.0).expect("resources are already shared");
        let id = TypeId::of::<R>();
        resources.entry(id).or_insert_with(|| Arc::new(f()));

        Arc::get_mut(resources.get_mut(&id).unwrap())
            .unwrap()
            .downcast_mut::<R>()
            .unwrap()
    }
}

#[derive(Debug)]
pub struct Res<T>(Arc<T>);

impl<T> Res<T> {
    pub(crate) fn new(inner: Arc<T>) -> Self {
        Self(inner)
    }
}

impl<T> Clone for Res<T> {
    fn clone(&self) -> Self {
        Res(self.0.clone())
    }
}

impl<T> Deref for Res<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> TaskParam for Res<T>
where
    T: Send + Sync + 'static,
{
    fn extract(context: &TaskContext) -> Result<Self> {
        Ok(Res(context.resources().get::<T>()?))
    }
}
