use crate::{
    error::{Error, Result},
    resource::Res,
    system::{SystemContext, SystemParam},
};
use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
    error, fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Eq)]
struct QueryId {
    req: TypeId,
    res: TypeId,
}

impl QueryId {
    fn of<Req, Resp>() -> Self
    where
        Req: 'static,
        Resp: 'static,
    {
        Self {
            req: TypeId::of::<Req>(),
            res: TypeId::of::<Resp>(),
        }
    }
}

impl PartialEq for QueryId {
    fn eq(&self, other: &Self) -> bool {
        self.req == other.req && self.res == other.res
    }
}

impl Hash for QueryId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.req.hash(state);
        self.res.hash(state);
    }
}

/// Sends query requests and waits for responses.
pub struct Querier<Req, Resp> {
    tx: flume::Sender<Request<Req, Resp>>,
}

impl<Req, Resp> Clone for Querier<Req, Resp> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<Req, Resp> Querier<Req, Resp>
where
    Req: Send + 'static,
    Resp: Send + 'static,
{
    /// Sends a query request and waits for its response.
    ///
    /// For bounded queries, this waits until the request queue has capacity.
    pub async fn query(&self, request: Req) -> std::result::Result<Resp, QueryError<Req>> {
        let (reply_to, reply_from) = oneshot::channel();
        self.tx
            .send_async(Request {
                data: request,
                reply_to,
            })
            .await
            .map_err(|error| QueryError::Closed(error.0.data))?;

        reply_from.await.map_err(|_| QueryError::ResponseDropped)
    }

    /// Attempts to send a query request without waiting, then waits for its response.
    pub async fn try_query(&self, request: Req) -> std::result::Result<Resp, TryQueryError<Req>> {
        let (reply_to, reply_from) = oneshot::channel();
        self.tx
            .try_send(Request {
                data: request,
                reply_to,
            })
            .map_err(|error| match error {
                flume::TrySendError::Full(request) => TryQueryError::Full(request.data),
                flume::TrySendError::Disconnected(request) => TryQueryError::Closed(request.data),
            })?;

        reply_from
            .await
            .map_err(|_| TryQueryError::ResponseDropped)
    }
}

/// Receives query requests.
pub struct Query<Req, Resp> {
    rx: flume::Receiver<Request<Req, Resp>>,
}

impl<Req, Resp> Clone for Query<Req, Resp> {
    fn clone(&self) -> Self {
        Self {
            rx: self.rx.clone(),
        }
    }
}

impl<Req, Resp> Query<Req, Resp>
where
    Req: Send + 'static,
    Resp: Send + 'static,
{
    /// Receives the next query request.
    pub async fn recv(&self) -> std::result::Result<Request<Req, Resp>, QueryRecvError> {
        self.rx.recv_async().await.map_err(|_| QueryRecvError::Closed)
    }

    /// Attempts to receive a query request without waiting.
    pub fn try_recv(&self) -> std::result::Result<Request<Req, Resp>, QueryTryRecvError> {
        self.rx.try_recv().map_err(Into::into)
    }
}

/// A received query request.
pub struct Request<Req, Resp> {
    pub data: Req,
    reply_to: oneshot::Sender<Resp>,
}

impl<Req, Resp> Request<Req, Resp> {
    /// Replies to this request.
    pub fn reply(self, response: Resp) -> std::result::Result<(), ReplyError<Resp>> {
        self.reply_to
            .send(response)
            .map_err(ReplyError::ResponseDropped)
    }

    /// Attempts to reply to this request.
    ///
    /// This is equivalent to `reply`; oneshot replies never wait.
    pub fn try_reply(self, response: Resp) -> std::result::Result<(), ReplyError<Resp>> {
        self.reply(response)
    }
}

struct QueryChannel<Req, Resp> {
    tx: flume::Sender<Request<Req, Resp>>,
    rx: flume::Receiver<Request<Req, Resp>>,
}

#[derive(Clone, Default)]
pub(crate) struct Queries(Arc<HashMap<QueryId, Box<dyn Any + Send + Sync>>>);

impl Queries {
    pub(crate) fn register<Req, Resp>(
        &mut self,
        channel: (
            flume::Sender<Request<Req, Resp>>,
            flume::Receiver<Request<Req, Resp>>,
        ),
    ) where
        Req: Send + 'static,
        Resp: Send + 'static,
    {
        let queries = Arc::get_mut(&mut self.0).expect("queries are already shared");
        let id = QueryId::of::<Req, Resp>();
        if queries
            .insert(
                id,
                Box::new(QueryChannel {
                    tx: channel.0,
                    rx: channel.1,
                }),
            )
            .is_some()
        {
            panic!(
                "{} -> {} is already registered as a query. Cannot register the same query type twice.",
                type_name::<Req>(),
                type_name::<Resp>(),
            );
        }
    }

    pub(crate) fn querier<Req, Resp>(&self) -> Result<Querier<Req, Resp>>
    where
        Req: Send + 'static,
        Resp: Send + 'static,
    {
        Ok(Querier {
            tx: self.query::<Req, Resp>().map_err(Error::source)?.tx.clone(),
        })
    }

    pub(crate) fn query_receiver<Req, Resp>(&self) -> Result<Query<Req, Resp>>
    where
        Req: Send + 'static,
        Resp: Send + 'static,
    {
        Ok(Query {
            rx: self.query::<Req, Resp>().map_err(Error::source)?.rx.clone(),
        })
    }

    fn query<Req, Resp>(&self) -> std::result::Result<&QueryChannel<Req, Resp>, QueryRegistryError>
    where
        Req: Send + 'static,
        Resp: Send + 'static,
    {
        self.0
            .get(&QueryId::of::<Req, Resp>())
            .ok_or(QueryRegistryError::Missing {
                request_type: type_name::<Req>(),
                response_type: type_name::<Resp>(),
            })?
            .downcast_ref::<QueryChannel<Req, Resp>>()
            .ok_or(QueryRegistryError::TypeMismatch {
                request_type: type_name::<Req>(),
                response_type: type_name::<Resp>(),
            })
    }
}

impl<Req, Resp> SystemParam for Querier<Req, Resp>
where
    Req: Send + 'static,
    Resp: Send + 'static,
{
    fn extract(context: &SystemContext) -> Result<Self> {
        Res::<Queries>::extract(context)?.querier::<Req, Resp>()
    }
}

impl<Req, Resp> SystemParam for Query<Req, Resp>
where
    Req: Send + 'static,
    Resp: Send + 'static,
{
    fn extract(context: &SystemContext) -> Result<Self> {
        Res::<Queries>::extract(context)?.query_receiver::<Req, Resp>()
    }
}

#[derive(Debug)]
pub enum QueryRegistryError {
    Missing {
        request_type: &'static str,
        response_type: &'static str,
    },
    TypeMismatch {
        request_type: &'static str,
        response_type: &'static str,
    },
}

impl fmt::Display for QueryRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing {
                request_type,
                response_type,
            } => {
                write!(
                    f,
                    "{request_type} -> {response_type} is not registered as a query"
                )
            }
            Self::TypeMismatch {
                request_type,
                response_type,
            } => {
                write!(
                    f,
                    "{request_type} -> {response_type} query had an unexpected type"
                )
            }
        }
    }
}

impl error::Error for QueryRegistryError {}

pub enum QueryError<Req> {
    Closed(Req),
    ResponseDropped,
}

pub enum TryQueryError<Req> {
    Full(Req),
    Closed(Req),
    ResponseDropped,
}

#[derive(Debug)]
pub enum QueryRecvError {
    Closed,
}

#[derive(Debug)]
pub enum QueryTryRecvError {
    Empty,
    Closed,
}

pub enum ReplyError<Resp> {
    ResponseDropped(Resp),
}

impl<Req> fmt::Debug for QueryError<Req> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(_) => f.debug_tuple("Closed").field(&"<request>").finish(),
            Self::ResponseDropped => f.debug_tuple("ResponseDropped").finish(),
        }
    }
}

impl<Req> fmt::Display for QueryError<Req> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(_) => write!(f, "query is closed"),
            Self::ResponseDropped => write!(f, "query response was dropped"),
        }
    }
}

impl<Req> error::Error for QueryError<Req> where Req: Send + Sync + 'static {}

impl<Req> fmt::Debug for TryQueryError<Req> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => f.debug_tuple("Full").field(&"<request>").finish(),
            Self::Closed(_) => f.debug_tuple("Closed").field(&"<request>").finish(),
            Self::ResponseDropped => f.debug_tuple("ResponseDropped").finish(),
        }
    }
}

impl<Req> fmt::Display for TryQueryError<Req> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => write!(f, "query is full"),
            Self::Closed(_) => write!(f, "query is closed"),
            Self::ResponseDropped => write!(f, "query response was dropped"),
        }
    }
}

impl<Req> error::Error for TryQueryError<Req> where Req: Send + Sync + 'static {}

impl From<flume::TryRecvError> for QueryTryRecvError {
    fn from(error: flume::TryRecvError) -> Self {
        match error {
            flume::TryRecvError::Empty => Self::Empty,
            flume::TryRecvError::Disconnected => Self::Closed,
        }
    }
}

impl fmt::Display for QueryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "query is closed")
    }
}

impl error::Error for QueryRecvError {}

impl fmt::Display for QueryTryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "query is empty"),
            Self::Closed => write!(f, "query is closed"),
        }
    }
}

impl error::Error for QueryTryRecvError {}

impl<Resp> fmt::Debug for ReplyError<Resp> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResponseDropped(_) => f
                .debug_tuple("ResponseDropped")
                .field(&"<response>")
                .finish(),
        }
    }
}

impl<Resp> fmt::Display for ReplyError<Resp> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "query response receiver was dropped")
    }
}

impl<Resp> error::Error for ReplyError<Resp> where Resp: Send + Sync + 'static {}
