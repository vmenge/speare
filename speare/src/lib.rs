use flume::{Receiver, Sender};
use futures_core::Stream;
use std::any::Any;
use std::{
    cmp,
    collections::HashMap,
    future::Future,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    task::{self, JoinSet},
    time,
};

mod exit;
mod node;
mod req_res;
mod streams;
mod watch;

pub use exit::*;
pub use node::*;
pub use req_res::*;
pub use streams::{SourceSet, Sources};

use crate::watch::{NoWatch, OnErrTerminate, WatchFn};

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

    /// Constructs the actor. Called on initial spawn and on every restart.
    ///
    /// # Example
    /// ```ignore
    /// async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///     Ok(MyActor { count: ctx.props().initial })
    /// }
    /// ```
    fn init(ctx: &mut Ctx<Self>) -> impl Future<Output = Result<Self, Self::Err>> + Send;

    /// Cleanup hook called when the actor stops, restarts, or fails to init.
    /// `this` is `None` if init failed.
    ///
    /// # Example
    /// ```ignore
    /// async fn exit(this: Option<Self>, reason: ExitReason<Self>, ctx: &mut Ctx<Self>) {
    ///     if let ExitReason::Err(e) = reason {
    ///         eprintln!("actor failed: {e:?}");
    ///     }
    /// }
    /// ```
    fn exit(
        this: Option<Self>,
        reason: ExitReason<Self>,
        ctx: &mut Ctx<Self>,
    ) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Sets up message sources (streams, intervals) after init.
    ///
    /// Sources added earlier in the [`SourceSet`] chain have higher polling priority.
    /// If an earlier source is consistently ready, later sources may be starved.
    ///
    /// # Example
    /// ```ignore
    /// async fn sources(&self, ctx: &Ctx<Self>) -> Result<impl Sources<Self>, Self::Err> {
    ///     Ok(SourceSet::new()
    ///         .interval(time::interval(Duration::from_millis(100)), || Msg::Tick)
    ///         .stream(my_stream))
    /// }
    /// ```
    fn sources(
        &self,
        ctx: &Ctx<Self>,
    ) -> impl Future<Output = Result<impl Sources<Self>, Self::Err>> + Send {
        async { Ok(SourceSet::new()) }
    }

    /// Called everytime your [`Actor`] receives a message.
    ///
    /// # Example
    /// ```ignore
    /// async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
    ///     match msg {
    ///         Msg::Inc(n) => self.count += n,
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn handle(
        &mut self,
        msg: Self::Msg,
        ctx: &mut Ctx<Self>,
    ) -> impl Future<Output = Result<(), Self::Err>> + Send {
        async { Ok(()) }
    }
}

/// A handle to send messages to or stop an [`Actor`].
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
    /// Stops the [`Actor`] associated with this handle. Does not wait for the actor to finish.
    ///
    /// # Example
    /// ```ignore
    /// handle.stop();
    /// ```
    pub fn stop(&self) {
        let (tx, _) = flume::unbounded();
        let _ = self
            .proc_msg_tx
            .send(ProcMsg::FromHandle(ProcAction::Stop(tx)));
    }

    /// Restarts the [`Actor`] by re-running [`Actor::init`] and [`Actor::sources`]. Does not wait for the actor to finish.
    ///
    /// # Example
    /// ```ignore
    /// handle.restart();
    /// ```
    pub fn restart(&self) {
        let _ = self
            .proc_msg_tx
            .send(ProcMsg::FromHandle(ProcAction::Restart));
    }

    /// Returns `true` if the [`Actor`] is still running.
    ///
    /// # Example
    /// ```ignore
    /// if handle.is_alive() {
    ///     handle.send(Msg::Ping);
    /// }
    /// ```
    pub fn is_alive(&self) -> bool {
        !self.msg_tx.is_disconnected()
    }

    /// Sends a message to the [`Actor`], failing silently if it is no longer running.
    /// Takes advantage of `From<_>` implementations on the message type.
    ///
    /// # Example
    /// ```ignore
    /// // Given `#[derive(From)] enum Msg { Inc(u32) }`:
    /// handle.send(Msg::Inc(1));
    /// handle.send(1u32); // works via From<u32>
    /// ```
    pub fn send<M: Into<Msg>>(&self, msg: M) {
        let _ = self.msg_tx.send(msg.into());
    }

    /// Sends a message to the [`Actor`] after the given duration, failing silently if it is no longer running.
    ///
    /// # Example
    /// ```ignore
    /// handle.send_in(Msg::Timeout, Duration::from_secs(5));
    /// ```
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

    /// Sends a request and awaits a response. Requires `Msg: From<Request<Req, Res>>`.
    ///
    /// # Example
    /// ```ignore
    /// #[derive(From)]
    /// enum Msg {
    ///     GetCount(Request<(), u32>),
    /// }
    ///
    /// // sender side:
    /// let count: u32 = handle.req(()).await?;
    ///
    /// // receiver side, inside handle():
    /// Msg::GetCount(req) => req.reply(self.count),
    /// ```
    pub async fn req<Req, Res>(&self, req: Req) -> Result<Res, ReqErr>
    where
        Msg: From<Request<Req, Res>>,
    {
        let (req, res) = req_res(req);
        self.send(req);
        res.recv().await
    }

    /// Like [`Handle::req`], but uses a wrapper function to convert the [`Request`] into the message type.
    /// Useful when the message variant can't implement `From<Request<Req, Res>>`.
    ///
    /// # Example
    /// ```ignore
    /// enum Msg {
    ///     GetCount(Request<(), u32>),
    /// }
    ///
    /// let count: u32 = handle.reqw(Msg::GetCount, ()).await?;
    /// ```
    pub async fn reqw<F, Req, Res>(&self, to_req: F, req: Req) -> Result<Res, ReqErr>
    where
        F: Fn(Request<Req, Res>) -> Msg,
    {
        let (req, res) = req_res(req);
        let msg = to_req(req);
        self.send(msg);
        res.recv().await
    }

    /// Like [`Handle::req`], but fails with [`ReqErr::Timeout`] if no response within the given [`Duration`].
    ///
    /// # Example
    /// ```ignore
    /// let count: u32 = handle.req_timeout((), Duration::from_secs(1)).await?;
    /// ```
    pub async fn req_timeout<Req, Res>(&self, req: Req, timeout: Duration) -> Result<Res, ReqErr>
    where
        Msg: From<Request<Req, Res>>,
    {
        let (req, res) = req_res(req);
        self.send(req);
        res.recv_timeout(timeout).await
    }

    /// Like [`Handle::reqw`], but fails with [`ReqErr::Timeout`] if no response within the given [`Duration`].
    ///
    /// # Example
    /// ```ignore
    /// let count: u32 = handle.reqw_timeout(Msg::GetCount, (), Duration::from_secs(1)).await?;
    /// ```
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
    registry_key: Option<String>,
    registry: Arc<RwLock<HashMap<String, Box<dyn Any + Send + Sync>>>>,
}

impl<P> Ctx<P>
where
    P: Actor,
{
    /// Returns a reference to this [`Actor`]'s props. Props are set once at spawn time
    /// and remain immutable for the lifetime of the actor, including across restarts.
    ///
    /// # Example
    /// ```ignore
    /// async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///     Ok(MyActor { count: ctx.props().initial_count })
    /// }
    /// ```
    pub fn props(&self) -> &P::Props {
        &self.props
    }

    /// Returns a [`Handle`] to the current [`Actor`], allowing it to send messages to itself
    /// or pass its handle to child actors.
    ///
    /// # Example
    /// ```ignore
    /// // schedule a message to self
    /// ctx.this().send_in(Msg::Tick, Duration::from_secs(1));
    /// ```
    pub fn this(&self) -> &Handle<P::Msg> {
        &self.handle
    }

    /// Drains all pending messages from this [`Actor`]'s mailbox. Useful during
    /// restarts to discard stale messages via [`Actor::init`].
    ///
    /// # Example
    /// ```ignore
    /// async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///     ctx.clear_mailbox();
    ///     Ok(MyActor::default())
    /// }
    /// ```
    pub fn clear_mailbox(&self) {
        self.msg_rx.drain();
    }

    /// Creates a [`SpawnBuilder`] for spawning a child [`Actor`]. The actor type is passed
    /// as a generic parameter and its props as the argument. The child is supervised
    /// by the current actor and will be stopped when the parent stops.
    ///
    /// # Example
    /// ```ignore
    /// let handle = ctx.actor::<Worker>(WorkerProps { id: 1 })
    ///     .supervision(Supervision::Restart {
    ///         max: Limit::Amount(3),
    ///         backoff: Backoff::None,
    ///     })
    ///     .spawn();
    /// ```
    pub fn actor<'a, Child>(&'a mut self, props: Child::Props) -> SpawnBuilder<'a, P, Child>
    where
        Child: Actor,
    {
        SpawnBuilder::new(self, props)
    }

    /// Restarts all child actors immediately, bypassing their supervision strategy.
    /// Each child will re-run its [`Actor::init`] with the same props.
    ///
    /// This is fire-and-forget: it does not wait for children to finish restarting.
    pub fn restart_children(&self) {
        for child in self.children_proc_msg_tx.values() {
            let _ = child.send(ProcMsg::FromParent(ProcAction::Restart));
        }
    }

    /// Stops all child actors and waits for each to fully terminate before returning.
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

    /// Spawns a background async task. On completion, its `Ok` value is delivered
    /// as a message to this [`Actor`]; its `Err` triggers the supervision strategy
    /// that this actor's parent has set for it.
    ///
    /// # Example
    /// ```ignore
    /// ctx.task(async {
    ///     let data = reqwest::get("https://example.com").await?.text().await?;
    ///     Ok(Msg::Fetched(data))
    /// });
    /// ```
    pub fn task<F>(&mut self, f: F)
    where
        F: Future<Output = Result<P::Msg, P::Err>> + Send + 'static,
    {
        self.tasks.spawn(f);
    }

    /// Looks up a registered [`Actor`]'s [`Handle`] by its type. The actor must have been
    /// spawned with [`SpawnBuilder::spawn_registered`].
    ///
    /// # Example
    /// ```ignore
    /// let logger = ctx.get_handle_for::<Logger>()?;
    /// logger.send(LogMsg::Info("hello".into()));
    /// ```
    pub fn get_handle_for<A: Actor>(&self) -> Result<Handle<A::Msg>, RegistryError> {
        let key = std::any::type_name::<A>();
        let reg = self.registry.read().map_err(|_| RegistryError::PoisonErr)?;
        reg.get(key)
            .and_then(|h| h.downcast_ref::<Handle<A::Msg>>())
            .cloned()
            .ok_or_else(|| RegistryError::NotFound(key.to_string()))
    }

    /// Looks up a registered [`Actor`]'s [`Handle`] by name. The actor must have been
    /// spawned with [`SpawnBuilder::spawn_named`].
    ///
    /// # Example
    /// ```ignore
    /// let worker = ctx.get_handle::<WorkerMsg>("worker-1")?;
    /// worker.send(WorkerMsg::Start);
    /// ```
    pub fn get_handle<Msg: Send + 'static>(
        &self,
        name: &str,
    ) -> Result<Handle<Msg>, RegistryError> {
        let reg = self.registry.read().map_err(|_| RegistryError::PoisonErr)?;
        reg.get(name)
            .and_then(|h| h.downcast_ref::<Handle<Msg>>())
            .cloned()
            .ok_or_else(|| RegistryError::NotFound(name.to_string()))
    }

    /// Sends a message to a registered [`Actor`] looked up by type.
    ///
    /// # Example
    /// ```ignore
    /// // Assuming MetricsCollector was spawned with spawn_registered():
    /// // ctx.actor::<MetricsCollector>(props).spawn_registered()?;
    ///
    /// // Any actor in the system can then send to it by type:
    /// ctx.send::<MetricsCollector>(MetricsMsg::RecordLatency(42))?;
    /// ```
    pub fn send<A: Actor>(&self, msg: impl Into<A::Msg>) -> Result<(), RegistryError> {
        let key = std::any::type_name::<A>();
        let reg = self.registry.read().map_err(|_| RegistryError::PoisonErr)?;
        match reg
            .get(key)
            .and_then(|h| h.downcast_ref::<Handle<A::Msg>>())
        {
            Some(handle) => {
                handle.send(msg);
                Ok(())
            }
            None => Err(RegistryError::NotFound(key.to_string())),
        }
    }

    /// Sends a message to a registered [`Actor`] looked up by name.
    ///
    /// # Example
    /// ```ignore
    /// // Assuming a Worker was spawned with spawn_named():
    /// // ctx.actor::<Worker>(props).spawn_named("worker-1")?;
    ///
    /// // Any actor in the system can then send to it by name:
    /// ctx.send_to::<WorkerMsg>("worker-1", WorkerMsg::Start)?;
    /// ```
    pub fn send_to<Msg: Send + 'static>(
        &self,
        name: &str,
        msg: impl Into<Msg>,
    ) -> Result<(), RegistryError> {
        let reg = self.registry.read().map_err(|_| RegistryError::PoisonErr)?;
        match reg.get(name).and_then(|h| h.downcast_ref::<Handle<Msg>>()) {
            Some(handle) => {
                handle.send(msg);
                Ok(())
            }
            None => Err(RegistryError::NotFound(name.to_string())),
        }
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

fn spawn<Child, W>(mut ctx: Ctx<Child>, delay: Option<Duration>, watch: W)
where
    Child: Actor,
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

            Ok(mut actor) => match actor.sources(&ctx).await {
                Err(e) => {
                    exit_reason = Some(ExitReason::Err(e));
                    restart = Restart::from_supervision(ctx.supervision, ctx.restarts);
                    actor_created = Some(actor);
                }

                Ok(mut sources) => {
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


                                    Ok(ProcMsg::FromHandle(ProcAction::Restart)) => {
                                        exit_reason = exit_reason.or(Some(ExitReason::Handle));
                                        restart = Restart::In(Duration::ZERO);
                                        break;
                                    }

                                    Ok(ProcMsg::ChildTerminated { child_id, }) => {
                                        if ctx.children_proc_msg_tx.remove(&child_id).is_some() {
                                            ctx.total_children -= 1;
                                        }
                                    }
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

                            Some(msg) = std::future::poll_fn(|cx| Pin::new(&mut sources).poll_next(cx)) => {
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
            },
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
            spawn::<Child, W>(ctx, Some(duration), watch)
        } else if let Some(parent_tx) = ctx.parent_proc_msg_tx {
            if let Some(key) = ctx.registry_key.take() {
                if let Ok(mut reg) = ctx.registry.write() {
                    reg.remove(&key);
                }
            }

            let _ = parent_tx.send(ProcMsg::ChildTerminated { child_id: ctx.id });
        }
    });
}

/// Defines how a parent reacts when a child actor fails.
///
/// # Example
/// ```ignore
/// let supervision = Supervision::Restart {
///     max: Limit::Amount(5),
///     backoff: Backoff::Static(Duration::from_secs(1)),
/// };
/// ```
#[derive(Debug, Clone, Copy)]
pub enum Supervision {
    /// Actor terminates on error.
    Stop,
    /// Actor continues processing the next message after an error.
    Resume,
    /// Actor is restarted on error, up to `max` times with optional `backoff`.
    Restart { max: Limit, backoff: Backoff },
}

/// Delay strategy between restart attempts.
///
/// # Example
/// ```ignore
/// let backoff = Backoff::Incremental {
///     min: Duration::from_millis(100),
///     max: Duration::from_secs(5),
///     step: Duration::from_millis(500),
/// };
/// ```
#[derive(Debug, Clone, Copy)]
pub enum Backoff {
    /// Restart immediately with no delay.
    None,
    /// Wait a fixed duration between restarts.
    Static(Duration),
    /// Linearly increase delay from `min` to `max` by `step` per restart.
    Incremental {
        min: Duration,
        max: Duration,
        step: Duration,
    },
}

/// Maximum number of restarts allowed.
///
/// # Example
/// ```ignore
/// let limit = Limit::Amount(3);
/// ```
#[derive(Debug, Clone, Copy)]
pub enum Limit {
    /// No limit on restarts.
    None,
    /// Restart at most this many times.
    Amount(u64),
}

/// **Note**: `0` maps to [`Limit::None`] (unlimited), not zero restarts.
/// If you want zero restarts (i.e., never restart), use [`Supervision::Stop`] instead.
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

#[derive(Debug)]
pub enum RegistryError {
    NameTaken(String),
    NotFound(String),
    PoisonErr,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::NameTaken(name) => write!(f, "registry name already taken: {name}"),
            RegistryError::NotFound(name) => write!(f, "no actor registered under: {name}"),
            RegistryError::PoisonErr => write!(f, "registry lock poisoned"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Builder for configuring and spawning a child [`Actor`]. Created via [`Ctx::actor`].
pub struct SpawnBuilder<'a, Parent, Child, W = NoWatch>
where
    Parent: Actor,
    Child: Actor,
{
    ctx: &'a mut Ctx<Parent>,
    props: Child::Props,
    supervision: Supervision,
    /// Only kicks in if child is stopped or reaches maximum number of restarts.
    watch: W,
    registry_key: Option<String>,
}

impl<'a, Parent, Child> SpawnBuilder<'a, Parent, Child, NoWatch>
where
    Parent: Actor,
    Child: Actor,
{
    fn new(ctx: &'a mut Ctx<Parent>, props: Child::Props) -> Self {
        Self {
            ctx,
            props,
            supervision: Supervision::Stop,
            watch: NoWatch,
            registry_key: None,
        }
    }
}

impl<'a, Parent, Child, W> SpawnBuilder<'a, Parent, Child, W>
where
    Parent: Actor,
    Child: Actor,
    W: OnErrTerminate<Child::Err>,
{
    /// Sets the [`Supervision`] strategy the parent will use for this child.
    /// Defaults to [`Supervision::Stop`].
    ///
    /// # Example
    /// ```ignore
    /// ctx.actor::<Worker>(props)
    ///     .supervision(Supervision::Restart {
    ///         max: Limit::Amount(3),
    ///         backoff: Backoff::Static(Duration::from_secs(1)),
    ///     })
    ///     .spawn();
    /// ```
    pub fn supervision(mut self, supervision: Supervision) -> Self {
        self.supervision = supervision;
        self
    }

    /// Registers a callback that fires when the child terminates due to an error.
    /// This happens when the supervision strategy is [`Supervision::Stop`], or when
    /// [`Supervision::Restart`] has exhausted all allowed restarts. The callback maps
    /// the child's error into a message for the parent.
    ///
    /// # Example
    /// ```ignore
    /// ctx.actor::<Worker>(props)
    ///     .supervision(Supervision::Restart {
    ///         max: Limit::Amount(3),
    ///         backoff: Backoff::None,
    ///     })
    ///     .watch(|err| ParentMsg::WorkerDied(format!("{err:?}")))
    ///     .spawn();
    /// ```
    pub fn watch<F>(self, f: F) -> SpawnBuilder<'a, Parent, Child, WatchFn<F, Parent::Msg>>
    where
        F: Fn(&Child::Err) -> Parent::Msg + Send + 'static,
    {
        let parent_msg_tx = self.ctx.handle.msg_tx.clone();
        SpawnBuilder {
            ctx: self.ctx,
            props: self.props,
            supervision: self.supervision,
            watch: WatchFn { f, parent_msg_tx },
            registry_key: self.registry_key,
        }
    }

    /// Spawns the child [`Actor`] and returns a [`Handle`] to it.
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
            registry_key: self.registry_key,
            registry: self.ctx.registry.clone(),
        };

        spawn::<Child, W>(ctx, None, self.watch);

        self.ctx
            .children_proc_msg_tx
            .insert(self.ctx.total_children, handle.proc_msg_tx.clone());

        handle
    }

    /// Spawns the child and registers it in the global registry under its type name.
    /// Other actors can then look it up via [`Ctx::get_handle_for`] or [`Ctx::send`].
    /// Returns [`RegistryError::NameTaken`] if already registered.
    pub fn spawn_registered(self) -> Result<Handle<Child::Msg>, RegistryError> {
        let key = std::any::type_name::<Child>();
        self.spawn_named(key)
    }

    /// Spawns the child and registers it in the global registry under the given name.
    /// Other actors can then look it up via [`Ctx::get_handle`] or [`Ctx::send_to`].
    /// Returns [`RegistryError::NameTaken`] if the name is already taken.
    ///
    /// # Example
    /// ```ignore
    /// let h = ctx.actor::<Worker>(props).spawn_named("worker-1")?;
    /// ```
    pub fn spawn_named(
        mut self,
        name: impl Into<String>,
    ) -> Result<Handle<Child::Msg>, RegistryError> {
        let name = name.into();
        let registry = self.ctx.registry.clone();
        let mut reg = registry.write().map_err(|_| RegistryError::PoisonErr)?;

        if reg.contains_key(&name) {
            return Err(RegistryError::NameTaken(name.clone()));
        }

        self.registry_key = Some(name.clone());
        let handle = self.spawn();
        reg.insert(name, Box::new(handle.clone()));

        Ok(handle)
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
                    Backoff::Static(duration) => duration,
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
