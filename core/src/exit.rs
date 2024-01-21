use std::{any::type_name, fmt, ops::Deref, sync::Arc};

use crate::Process;

/// A thin wrapper around `Arc<E>` with custom `fmt::Debug` and `fmt::Display`
/// implementations for better error logging.
pub struct SharedErr<E> {
    err: Arc<E>,
}

impl<E> SharedErr<E> {
    pub fn new(e: E) -> Self {
        Self { err: Arc::new(e) }
    }
}

impl<E> Clone for SharedErr<E> {
    fn clone(&self) -> Self {
        Self {
            err: self.err.clone(),
        }
    }
}

impl<E> Deref for SharedErr<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        let name = type_name::<E>();
        println!("deref called on {name}!");
        self.err.deref()
    }
}

impl<E> AsRef<E> for SharedErr<E> {
    fn as_ref(&self) -> &E {
        let name = type_name::<E>();
        println!("asref called on {name}!");
        self.err.as_ref()
    }
}

impl<E> fmt::Debug for SharedErr<E>
where
    E: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.err.as_ref())
    }
}

impl<E> fmt::Display for SharedErr<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.err.as_ref())
    }
}

/// Enumerates the reasons why a `Process` might exit.
pub enum ExitReason<P>
where
    P: Process,
{
    /// Process exited due to manual request through a `Handle<P>`
    Handle,
    /// Process exited due to a request from its Parent process as a part of its supervision strategy.
    Parent,
    /// Procss exited due to error.
    Err(SharedErr<P::Err>),
}

impl<P> fmt::Debug for ExitReason<P>
where
    P: Process,
    P::Err: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handle => write!(f, "ExitReason::Handle"),
            Self::Parent => write!(f, "ExitReason::Parent"),
            Self::Err(arg0) => write!(f, "ExitReason::Err({:?})", arg0),
        }
    }
}

impl<P> fmt::Display for ExitReason<P>
where
    P: Process,
    P::Err: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handle => write!(f, "manual exit through Handle::stop"),
            Self::Parent => write!(f, "exit request from parent supervision strategy"),
            Self::Err(e) => write!(f, "{e}"),
        }
    }
}
