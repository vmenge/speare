mod app;
mod channel;
mod scope;
mod error;
mod query;
mod resource;
mod retry;
mod shutdown;
mod system;

pub use app::{App, AppHandle, Schedule};
pub use channel::{
    BroadcastError, BroadcastRecvError, BroadcastTryRecvError, ChannelError, Pub, PublishError,
    Receiver, RecvError, SendError, Sender, Sub, TryRecvError, TrySendError,
};
pub use scope::{Child, ChildFailure, ChildId, Scope};
pub use error::{Error, IntoSystemResult, Result};
pub use query::{
    Querier, Query, QueryError, QueryRecvError, QueryRegistryError, QueryTryRecvError, ReplyError,
    Request, TryQueryError,
};
pub use resource::Res;
pub use retry::{
    Backoff, PolicySystem, RestartPolicy, Retry, RetrySystem, SystemExt,
};
pub use shutdown::Shutdown;
pub use system::{IntoSystem, SystemParam};

/// Common imports for application and system authoring.
///
/// Import this with `use rs::prelude::*;`.
pub mod prelude {
    pub use crate::{
        App, AppHandle, Backoff, BroadcastRecvError, BroadcastTryRecvError, ChannelError, Child,
        ChildFailure, ChildId, Error, IntoSystem, IntoSystemResult, PolicySystem, Pub,
        PublishError, Querier, Query, QueryError, QueryRecvError, QueryRegistryError,
        QueryTryRecvError, Receiver, RecvError, ReplyError, Request, Res, RestartPolicy, Result,
        Retry, RetrySystem, Schedule, Scope, SendError, Sender, Shutdown, Sub, SystemExt,
        SystemParam, TryQueryError, TryRecvError, TrySendError,
    };
}
