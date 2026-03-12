use flume::{Receiver, Sender};
use futures_core::Stream;
use std::{cmp, collections::HashMap, future::Future, marker::PhantomData, time::Duration};
use tokio::{
    task::{self, JoinSet},
    time::{self, Interval},
};

mod exit;
mod req_res;
mod streams;
mod watch;

pub use exit::*;
pub use req_res::*;

use crate::{
    streams::{IntervalStream, Merge, NoStream},
    watch::{NoWatch, OnErrTerminate, WatchFn},
};

/// A thin abstraction over tokio tasks and flume channels, allowing for easy message passing
/// with a supervision tree to handle failures.
///
/// ## Example
/// ```
/// use speare::{Ctx, Actor};
/// use derive_more::From;
///
/// struct Counter {
///     count: u32,
/// }
///
/// struct CounterProps {
///     initial_count: u32,
///     max_count: u32,
/// }
///
/// #[derive(From)]
/// enum CounterMsg {
///     Inc(u32),
/// }
///
/// enum CounterErr {
///     MaxCountExceeded,
/// }
///
/// impl Actor for Counter {
///     type Props = CounterProps;
///     type Msg = CounterMsg;
///     type Err = CounterErr;
///
///     async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
///         Ok(Counter {
///             count: ctx.props().initial_count,
///         })
///     }
///
///     async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
///         match msg {
///             CounterMsg::Inc(x) => {
///                 self.count += x;
///
///                 if self.count > ctx.props().max_count {
///                     return Err(CounterErr::MaxCountExceeded);
///                 }
///             }
///         }
///
///         Ok(())
///     }
/// }
/// ```
#[allow(unused_variables)]
pub trait Actor: Sized + Send + 'static {
    type Props: Send + 'static;
    type Msg: Send + 'static;
    type Err: Send + Sync + 'static;

    /// The constructor function that will be used to create an instance of your [`Actor`]
    /// when spawning or restarting it.
    fn init(ctx: &mut Ctx<Self>) -> impl Future<Output = Result<Self, Self::Err>> + Send;

    /// A function that will be called if your [`Actor`] fails to init, is stopped or restarted.
    ///
    /// `this` is `None` if the [`Actor`] is failing on `init`.
    fn exit(
        this: Option<Self>,
        reason: ExitReason<Self>,
        ctx: &mut Ctx<Self>,
    ) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Called everytime your [`Actor`] receives a message.
    fn handle(
        &mut self,
        msg: Self::Msg,
        ctx: &mut Ctx<Self>,
    ) -> impl Future<Output = Result<(), Self::Err>> + Send {
        async { Ok(()) }
    }
}

/// A handle to send messages to or stop a `Proccess`.
pub struct Handle<Msg> {
    msg_tx: Sender<Msg>,
    proc_msg_tx: Sender<ProcMsg>,
}

impl<Msg> Clone for Handle<Msg> {
    fn clone(&self) -> Self {
        Self {
            msg_tx: self.msg_tx.clone(),
            proc_msg_tx: self.proc_msg_tx.clone(),
        }
    }
}

impl<Msg> Handle<Msg> {
    /// Stops the [`Actor`] for which this `Handle<_>` is for.
    pub fn stop(&self) {
        let (tx, _) = flume::unbounded();
        let _ = self
            .proc_msg_tx
            .send(ProcMsg::FromHandle(ProcAction::Stop(tx)));
    }

    pub fn restart(&self) {
        let _ = self
            .proc_msg_tx
            .send(ProcMsg::FromHandle(ProcAction::Restart));
    }

    /// Returns true if the [`Actor`] is still running, false if it has been stopped.
    pub fn is_alive(&self) -> bool {
        !self.msg_tx.is_disconnected()
    }

    /// Sends a message to the [`Actor`] associated with this `Handle<_>`, failing silently if that [`Actor`] is no longer running.
    ///
    /// `send` can take advantage of `From<_>` implementations for the variants of the `Actor::Msg` type.
    pub fn send<M: Into<Msg>>(&self, msg: M) {
        let _ = self.msg_tx.send(msg.into());
    }

    /// After the given duration, sends a message to the `Actor ` associated with this `Handle<_>`, failing silently if that [`Actor`] is no longer running.
    pub fn send_in<M>(&self, msg: M, duration: Duration)
    where
        Msg: 'static + Send,
        M: 'static + Send + Into<Msg>,
    {
        let msg_tx = self.msg_tx.clone();

        task::spawn(async move {
            time::sleep(duration).await;
            let _ = msg_tx.send(msg.into());
        });
    }

    /// Sends a request to the `Actor ` as long as its messages implements `From<Request<Req,Res>>`.
    ///
    /// In `speare` a `Request<Req,Res>` allows a request-response transaction between actors.
    pub async fn req<Req, Res>(&self, req: Req) -> Result<Res, ReqErr>
    where
        Msg: From<Request<Req, Res>>,
    {
        let (req, res) = req_res(req);
        self.send(req);
        res.recv().await
    }

    pub async fn reqw<F, Req, Res>(&self, to_req: F, req: Req) -> Result<Res, ReqErr>
    where
        F: Fn(Request<Req, Res>) -> Msg,
    {
        let (req, res) = req_res(req);
        let msg = to_req(req);
        self.send(msg);
        res.recv().await
    }

    /// Sends a request to the `Actor ` as long as its messages implements `From<Request<Req,Res>>`.
    ///
    /// Fails if response is not sent back within the given `Duration`.
    pub async fn req_timeout<Req, Res>(&self, req: Req, timeout: Duration) -> Result<Res, ReqErr>
    where
        Msg: From<Request<Req, Res>>,
    {
        let (req, res) = req_res(req);
        self.send(req);
        res.recv_timeout(timeout).await
    }

    pub async fn reqw_timeout<F, Req, Res>(
        &self,
        to_req: F,
        req: Req,
        timeout: Duration,
    ) -> Result<Res, ReqErr>
    where
        F: Fn(Request<Req, Res>) -> Msg,
    {
        let (req, res) = req_res(req);
        let msg = to_req(req);
        self.send(msg);
        res.recv_timeout(timeout).await
    }
}

/// The context surrounding the current `Actor`.
///
/// Provides a collection of methods that allow you to:
/// - spawn other actors as children of the current actor
/// - access the `Handle<_>` for the currrent actor
/// - access this actor's props
/// - clear this actor's mailbox
pub struct Ctx<P>
where
    P: Actor,
{
    id: u64,
    props: P::Props,
    handle: Handle<P::Msg>,
    msg_rx: Receiver<P::Msg>,
    parent_proc_msg_tx: Option<Sender<ProcMsg>>,
    proc_msg_rx: Receiver<ProcMsg>,
    children_proc_msg_tx: HashMap<u64, Sender<ProcMsg>>,
    supervision: Supervision,
    total_children: u64,
    tasks: JoinSet<Result<P::Msg, P::Err>>,
    restarts: u64,
}

impl<P> Ctx<P>
where
    P: Actor,
{
    /// Returns a reference to the `Actor::Props` of the current `Actor `.
    pub fn props(&self) -> &P::Props {
        &self.props
    }

    /// Returns a reference to a `Handle` of the current `Actor `.
    pub fn this(&self) -> &Handle<P::Msg> {
        &self.handle
    }

    /// Clears all the messages from the mailbox.
    pub fn clear_mailbox(&self) {
        self.msg_rx.drain();
    }

    /// Spawns and supervises a child `Actor `.
    pub fn actor<'a, Child>(&'a mut self, props: Child::Props) -> SpawnBuilder<'a, P, Child>
    where
        Child: Actor,
    {
        SpawnBuilder::new(self, props)
    }

    pub async fn stop_children(&mut self) {
        let mut acks = Vec::with_capacity(self.total_children as usize);
        for child in self.children_proc_msg_tx.values() {
            let (ack_tx, ack_rx) = flume::unbounded();
            let _ = child.send(ProcMsg::FromParent(ProcAction::Stop(ack_tx)));
            acks.push(ack_rx);
        }

        for ack in acks {
            let _ = ack.recv_async().await;
        }

        self.total_children = 0;
        self.children_proc_msg_tx.clear();
    }

    pub fn task<F>(&mut self, f: F)
    where
        F: Future<Output = Result<P::Msg, P::Err>> + Send + 'static,
    {
        self.tasks.spawn(f);
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
enum ProcMsg {
    /// Sent from child once it terminates
    ChildTerminated {
        child_id: u64,
    },
    FromParent(ProcAction),
    FromHandle(ProcAction),
}

#[derive(Debug)]
enum ProcAction {
    Restart,
    Stop(Sender<()>),
}

fn spawn<Child, S, W>(mut ctx: Ctx<Child>, delay: Option<Duration>, mut stream: S, watch: W)
where
    Child: Actor,
    S: Stream<Item = Child::Msg> + Send + Unpin + 'static,
    W: OnErrTerminate<Child::Err>,
{
    tokio::spawn(async move {
        if let Some(d) = delay.filter(|d| !d.is_zero()) {
            time::sleep(d).await;
        }

        // restart is Some whenever we should restart
        let mut restart = Restart::No;
        let mut exit_reason = None;
        let mut actor_created = None;
        let mut stop_ack_tx = None;

        match Child::init(&mut ctx).await {
            Err(e) => {
                exit_reason = Some(ExitReason::Err(e));
                restart = Restart::from_supervision(ctx.supervision, ctx.restarts);
            }

            Ok(mut actor) => {
                macro_rules! on_err {
                    ($e:expr) => {
                        if let Supervision::Resume = ctx.supervision {
                            continue;
                        }

                        restart = Restart::from_supervision(ctx.supervision, ctx.restarts);
                        exit_reason = Some(ExitReason::Err($e));
                        actor_created = Some(actor);
                        break;
                    };
                }

                loop {
                    tokio::select! {
                        biased;

                        proc_msg = ctx.proc_msg_rx.recv_async() => {
                            match proc_msg {
                                Err(_) => break,

                                Ok(ProcMsg::FromHandle(ProcAction::Stop(tx)) ) => {
                                    exit_reason = Some(ExitReason::Handle);
                                    stop_ack_tx = Some(tx);
                                    break
                                },

                                Ok(ProcMsg::FromParent(ProcAction::Stop(tx))) => {
                                    exit_reason = exit_reason.or(Some(ExitReason::Parent));
                                    stop_ack_tx = Some(tx);
                                    break
                                },

                                Ok(ProcMsg::FromParent(ProcAction::Restart)) => {
                                    exit_reason = exit_reason.or(Some(ExitReason::Parent));
                                    restart = Restart::In(Duration::ZERO);
                                    break;
                                }

                                Ok(ProcMsg::ChildTerminated { child_id, }) => {
                                    if ctx.children_proc_msg_tx.remove(&child_id).is_some() {
                                        ctx.total_children -= 1;
                                    }
                                }

                                Ok(_) => ()
                            }
                        }

                        Some(Ok(msg)) = ctx.tasks.join_next() => {
                            match msg {
                                Err(e) => {
                                    on_err!(e);
                                }

                                Ok(msg) => {
                                    if let Err(e) = actor.handle(msg, &mut ctx).await {
                                        on_err!(e);
                                    };
                                }
                            }

                        }

                        Some(msg) = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)) => {
                            if let Err(e) = actor.handle(msg, &mut ctx).await {
                                on_err!(e);
                            };
                        }

                        recvd = ctx.msg_rx.recv_async() => {
                            match recvd {
                                Err(_) => break,

                                Ok(msg) => {
                                    if let Err(e) = actor.handle(msg, &mut ctx).await {
                                        on_err!(e);
                                    };
                                }
                            }
                        }
                    }
                }
            }
        }

        ctx.stop_children().await;
        let exit_reason = exit_reason.unwrap_or(ExitReason::Handle);

        if let ExitReason::Err(_) = &exit_reason {
            ctx.restarts += 1;
        }

        if let (Restart::No, ExitReason::Err(ref e)) = (&restart, &exit_reason) {
            watch.on_err_terminate(e);
        }

        Child::exit(actor_created, exit_reason, &mut ctx).await;
        let _ = stop_ack_tx.map(|tx| tx.send(()));

        if let Restart::In(duration) = restart {
            spawn::<Child, S, W>(ctx, Some(duration), stream, watch)
        } else if let Some(parent_tx) = ctx.parent_proc_msg_tx {
            let _ = parent_tx.send(ProcMsg::ChildTerminated { child_id: ctx.id });
        }
    });
}

#[derive(Debug, Clone, Copy)]
pub enum Supervision {
    Stop,
    Resume,
    Restart { max: Limit, backoff: Backoff },
}

#[derive(Debug, Clone, Copy)]
pub enum Backoff {
    None,
    Satic(Duration),
    Incremental {
        min: Duration,
        max: Duration,
        step: Duration,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum Limit {
    None,
    Amount(u64),
}

impl From<u64> for Limit {
    fn from(value: u64) -> Self {
        match value {
            0 => Limit::None,
            v => Limit::Amount(v),
        }
    }
}

impl PartialEq<u64> for Limit {
    fn eq(&self, other: &u64) -> bool {
        match self {
            Limit::None => false,
            Limit::Amount(n) => n == other,
        }
    }
}

pub struct SpawnBuilder<'a, Parent, Child, S = NoStream<<Child as Actor>::Msg>, W = NoWatch>
where
    Parent: Actor,
    Child: Actor,
{
    ctx: &'a mut Ctx<Parent>,
    props: Child::Props,
    supervision: Supervision,
    /// Only kicks in if child is stopped or reaches maximum number of restarts.
    watch: W,
    stream: S,
}

impl<'a, Parent, Child> SpawnBuilder<'a, Parent, Child, NoStream<Child::Msg>, NoWatch>
where
    Parent: Actor,
    Child: Actor,
{
    fn new(ctx: &'a mut Ctx<Parent>, props: Child::Props) -> Self {
        Self {
            ctx,
            props,
            supervision: Supervision::Stop,
            stream: NoStream(PhantomData),
            watch: NoWatch,
        }
    }
}

impl<'a, Parent, Child, S, W> SpawnBuilder<'a, Parent, Child, S, W>
where
    Parent: Actor,
    Child: Actor,
    S: Stream<Item = Child::Msg> + Send + Unpin + 'static,
    W: OnErrTerminate<Child::Err>,
{
    pub fn supervision(mut self, supervision: Supervision) -> Self {
        self.supervision = supervision;
        self
    }

    pub fn interval<F>(
        self,
        interval: Interval,
        f: F,
    ) -> SpawnBuilder<'a, Parent, Child, Merge<S, IntervalStream<F>>, W>
    where
        F: Fn() -> Child::Msg + Send + Unpin + 'static,
    {
        self.stream(IntervalStream { interval, f })
    }

    pub fn watch<F>(self, f: F) -> SpawnBuilder<'a, Parent, Child, S, WatchFn<F, Parent::Msg>>
    where
        F: Fn(&Child::Err) -> Parent::Msg + Send + 'static,
    {
        let parent_msg_tx = self.ctx.handle.msg_tx.clone();
        SpawnBuilder {
            ctx: self.ctx,
            props: self.props,
            supervision: self.supervision,
            watch: WatchFn { f, parent_msg_tx },
            stream: self.stream,
        }
    }

    pub fn stream<S2>(self, stream: S2) -> SpawnBuilder<'a, Parent, Child, Merge<S, S2>, W>
    where
        S2: Stream<Item = Child::Msg> + Send + 'static,
    {
        SpawnBuilder {
            ctx: self.ctx,
            props: self.props,
            supervision: self.supervision,
            watch: self.watch,
            stream: Merge {
                a: self.stream,
                b: stream,
            },
        }
    }

    pub fn spawn(self) -> Handle<Child::Msg> {
        let (msg_tx, msg_rx) = flume::unbounded(); // child
        let (proc_msg_tx, proc_msg_rx) = flume::unbounded(); // child

        let handle = Handle {
            msg_tx,
            proc_msg_tx,
        };

        self.ctx.total_children += 1;
        let id = self.ctx.total_children;

        let ctx: Ctx<Child> = Ctx {
            id,
            props: self.props,
            handle: handle.clone(),
            msg_rx,
            parent_proc_msg_tx: Some(self.ctx.handle.proc_msg_tx.clone()),
            proc_msg_rx,
            children_proc_msg_tx: HashMap::new(),
            total_children: 0,
            supervision: self.supervision,
            restarts: 0,
            tasks: JoinSet::new(),
        };

        spawn::<Child, S, W>(ctx, None, self.stream, self.watch);

        self.ctx
            .children_proc_msg_tx
            .insert(self.ctx.total_children, handle.proc_msg_tx.clone());

        handle
    }
}

#[derive(Debug)]
enum Restart {
    No,
    In(Duration),
}

impl Restart {
    fn from_supervision(supervision: Supervision, current_restarts: u64) -> Self {
        match supervision {
            Supervision::Stop => Restart::No,
            Supervision::Resume => Restart::No,
            Supervision::Restart { max, .. } if max == current_restarts + 1 => Restart::No,
            Supervision::Restart { backoff, .. } => {
                let wait = match backoff {
                    Backoff::None => Duration::ZERO,
                    Backoff::Satic(duration) => duration,
                    Backoff::Incremental { min, max, step } => {
                        let wait = step.mul_f64((current_restarts + 1) as f64);
                        let wait = cmp::min(max, wait);
                        cmp::max(min, wait)
                    }
                };

                Restart::In(wait)
            }
        }
    }
}

pub struct Node {
    ctx: Ctx<Node>,
}

impl Actor for Node {
    type Props = ();
    type Msg = ();
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        unreachable!("how did you get here")
    }
}

impl Node {
    pub fn new() -> Self {
        let (msg_tx, msg_rx) = flume::unbounded(); // child
        let (proc_msg_tx, proc_msg_rx) = flume::unbounded(); // child

        let handle = Handle {
            msg_tx,
            proc_msg_tx,
        };

        let ctx = Ctx {
            id: 0,
            props: (),
            handle,
            msg_rx,
            parent_proc_msg_tx: None,
            proc_msg_rx,
            children_proc_msg_tx: HashMap::new(),
            total_children: 0,
            supervision: Supervision::Stop,
            restarts: 0,
            tasks: JoinSet::new(),
        };

        Self { ctx }
    }

    pub fn actor<'a, Child>(&'a mut self, props: Child::Props) -> SpawnBuilder<'a, Node, Child>
    where
        Child: Actor,
    {
        SpawnBuilder::new(&mut self.ctx, props)
    }

    /// Stops all children. (Drop impl is fire and forget)
    pub async fn shutdown(&mut self) {
        self.ctx.stop_children().await;
    }
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let mut acks = Vec::with_capacity(self.ctx.total_children as usize);
        for child in self.ctx.children_proc_msg_tx.values() {
            let (ack_tx, ack_rx) = flume::unbounded();
            let _ = child.send(ProcMsg::FromParent(ProcAction::Stop(ack_tx)));
            acks.push(ack_rx);
        }

        task::spawn(async {
            for ack in acks {
                let _ = ack.recv_async().await;
            }
        });
    }
}
