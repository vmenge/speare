use crate::Actor;
use std::fmt;

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
    Err(P::Err),
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
