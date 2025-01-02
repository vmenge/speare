use flume::{Receiver, Sender};
use std::{fmt, time::Duration};
use tokio::time;

/// Represents a request sent to a `Actor`.
/// `Request` holds the data sent to a `Actor` and provides a channel to reply back to the sender.
///
/// ## Example
/// ```
/// use speare::{req_res, Ctx, Node, Actor, Request};
/// use async_trait::async_trait;
/// use derive_more::From;
/// use tokio::runtime::Runtime;
///
/// Runtime::new().unwrap().block_on(async {
///     let node = Node::default();
///     let parser = node.spawn::<Parser>(());
///
///     // use Handle<_>::req if the Actor::Msg
///     // implements From<Request<_,_>>
///     let num = parser.req("5".to_string()).await.unwrap();
///     assert_eq!(num, 5);
///
///     // or manually create a Request<_>
///     let (req, res) = req_res("10".to_string());
///     parser.send(req);
///     let num = res.recv().await.unwrap();
///     assert_eq!(num, 10);
/// });
///
/// struct Parser;
///
/// #[derive(From)]
/// enum ParserMsg {
///     Parse(Request<String, u32>),
/// }
///
/// #[async_trait]
/// impl Actor for Parser {
///     type Props = ();
///     type Msg = ParserMsg;
///     type Err = ();
///
///     async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
///         Ok(Parser)
///     }
///
///     async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
///         match msg {
///             ParserMsg::Parse(req) => {
///                 let num = req.data().parse().unwrap_or(0);
///                 req.reply(num)
///             }
///         }
///
///         Ok(())
///     }
/// }
/// ```
pub struct Request<Req, Res> {
    data: Req,
    tx: Sender<Res>,
}

impl<Req, Res> fmt::Debug for Request<Req, Res>
where
    Req: fmt::Debug,
    Res: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("data", &self.data)
            .field("tx", &self.tx)
            .finish()
    }
}

impl<Req, Res> Request<Req, Res> {
    /// Returns a reference to the data inside the `Request`.
    pub fn data(&self) -> &Req {
        &self.data
    }

    /// Sends a response back to the requester.
    pub fn reply(&self, res: Res) {
        let _ = self.tx.send(res);
    }
}

///`Response<Res>` is used to asynchronously wait for and retrieve the result of a `Request<Req, Res>` sent to a `Actor`.
///
/// ## Example
/// ```
/// use speare::{req_res, Ctx, Node, Actor, Request};
/// use async_trait::async_trait;
/// use derive_more::From;
/// use tokio::runtime::Runtime;
///
/// Runtime::new().unwrap().block_on(async {
///     let node = Node::default();
///     let parser = node.spawn::<Parser>(());
///
///     let (req, res) = req_res("10".to_string());
///     parser.send(req);
///     let num = res.recv().await.unwrap();
///     assert_eq!(num, 10);
/// });
///
/// struct Parser;
///
/// #[derive(From)]
/// enum ParserMsg {
///     Parse(Request<String, u32>),
/// }
///
/// #[async_trait]
/// impl Actor for Parser {
///     type Props = ();
///     type Msg = ParserMsg;
///     type Err = ();
///
///     async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
///         Ok(Parser)
///     }
///
///     async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
///         match msg {
///             ParserMsg::Parse(req) => {
///                 let num = req.data().parse().unwrap_or(0);
///                 req.reply(num)
///             }
///         }
///
///         Ok(())
///     }
/// }
/// ```
pub struct Response<Res> {
    rx: Receiver<Res>,
}

impl<Res> fmt::Debug for Response<Res>
where
    Res: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response").field("rx", &self.rx).finish()
    }
}

#[derive(Debug)]
/// Represents a failure when waiting for a `Response<_>`
pub enum ReqErr {
    /// Request object was dropped before replying.
    Dropped,
    /// Timed out waiting for response.
    Timeout,
}

impl fmt::Display for ReqErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dropped => write!(f, "Request object dropped before replying."),
            Self::Timeout => write!(f, "Timed out before receiving response."),
        }
    }
}

impl std::error::Error for ReqErr {}

impl<Res> Response<Res> {
    /// Asynchronously wait for a value from this channel,
    /// returning an error if the corresponding `Request<_,_>` has been dropped.
    pub async fn recv(self) -> Result<Res, ReqErr> {
        self.rx.recv_async().await.map_err(|_| ReqErr::Dropped)
    }

    /// Asynchronously wait for a value from this channel,
    /// returning an error if the given `Duration` elapses before
    /// receiving the expected response, or if the corresponding `Request<_,_>` has been dropped.
    pub async fn recv_timeout(self, dur: Duration) -> Result<Res, ReqErr> {
        time::timeout(dur, self.recv())
            .await
            .map_err(|_| ReqErr::Timeout)
            .and_then(|x| x)
    }
}

/// Creates a paired `Request<Req, Res>` and `Response<Res>` for communication between `speare` actors.
/// ## Example
/// ```
/// use speare::{req_res, Ctx, Node, Actor, Request};
/// use async_trait::async_trait;
/// use derive_more::From;
/// use tokio::runtime::Runtime;
///
/// Runtime::new().unwrap().block_on(async {
///     let mut node = Node::default();
///     let parser = node.spawn::<Parser>(());
///
///     let (req, res) = req_res("10".to_string());
///     parser.send(req);
///     let num = res.recv().await.unwrap();
///     assert_eq!(num, 10);
/// });
///
/// struct Parser;
///
/// #[derive(From)]
/// enum ParserMsg {
///     Parse(Request<String, u32>),
/// }
///
/// #[async_trait]
/// impl Actor for Parser {
///     type Props = ();
///     type Msg = ParserMsg;
///     type Err = ();
///
///     async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
///         Ok(Parser)
///     }
///
///     async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
///         match msg {
///             ParserMsg::Parse(req) => {
///                 let num = req.data().parse().unwrap_or(0);
///                 req.reply(num)
///             }
///         }
///
///         Ok(())
///     }
/// }
/// ```
pub fn req_res<Req, Res>(req: Req) -> (Request<Req, Res>, Response<Res>) {
    let (tx, rx) = flume::unbounded();
    (Request { data: req, tx }, Response { rx })
}
