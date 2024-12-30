use std::{fmt, ops::Deref, sync::Arc};

use crate::Actor;

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
        self.err.deref()
    }
}

impl<E> AsRef<E> for SharedErr<E> {
    fn as_ref(&self) -> &E {
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

/// Enumerates the reasons why a [`Actor`] might exit.
pub enum ExitReason<P>
where
    P: Actor,
{
    /// [`Actor`] exited due to manual request through a `Handle<_>`
    Handle,
    /// [`Actor`] exited due to a request from its Parent [`Actor`] as a part of its supervision strategy.
    Parent,
    /// [`Actor`] exited due to error.
    Err(SharedErr<P::Err>),
}

impl<P> fmt::Debug for ExitReason<P>
where
    P: Actor,
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
    P: Actor,
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
