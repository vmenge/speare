mod app;
mod channel;
mod error;
mod query;
mod resource;
mod retry;
mod scope;
mod shutdown;
mod task;

pub use app::{App, AppBuilder, Plugin};
pub use channel::{
    AppChannelExt, BroadcastError, BroadcastRecvError, BroadcastTryRecvError, ChannelError, Pub,
    PublishError, Receiver, RecvError, SendError, Sender, Sub, TryRecvError, TrySendError,
};
pub use error::{Error, IntoTaskResult, Result, RunError};
pub use query::{
    AppQueryExt, Querier, Query, QueryError, QueryRecvError, QueryRegistryError, QueryTryRecvError,
    ReplyError, Request, TryQueryError,
};
pub use resource::Res;
pub use retry::{Backoff, PolicyTask, RestartPolicy, Retry, RetryTask, TaskExt};
pub use scope::{Child, ChildFailure, ChildId, Scope};
pub use shutdown::Shutdown;
pub use task::{IntoTask, TaskParam};

/// Common imports for application and task authoring.
///
/// Import this with `use speare::prelude::*;`.
pub mod prelude {
    pub use crate::{
        App, AppBuilder, AppChannelExt, AppQueryExt, Backoff, BroadcastRecvError,
        BroadcastTryRecvError, ChannelError, Child, ChildFailure, ChildId, Error, IntoTask,
        IntoTaskResult, Plugin, PolicyTask, Pub, PublishError, Querier, Query, QueryError,
        QueryRecvError, QueryRegistryError, QueryTryRecvError, Receiver, RecvError, ReplyError,
        Request, Res, RestartPolicy, Retry, RetryTask, Scope, SendError, Sender, Shutdown, Sub,
        TaskExt, TaskParam, TryQueryError, TryRecvError, TrySendError,
    };
}
