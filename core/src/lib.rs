use async_trait::async_trait;
use flume::{Receiver, Sender};
use std::{
    any::{type_name, Any},
    cmp,
    collections::HashMap,
    fmt,
    ops::Deref,
    sync::Arc,
    time::Duration,
};
use tokio::{
    task,
    time::{self, Instant},
};

mod req_res;

pub use req_res::*;

#[allow(unused_variables)]
#[async_trait]
pub trait Process: Sized + Send + 'static {
    type Props: Send + Sync + 'static;
    type Msg: Send + 'static;
    type Err: Send + Sync + 'static;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err>;

    async fn exit(&mut self, reason: ExitReason<Self>, ctx: &mut Ctx<Self>) {}

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        Ok(())
    }

    fn supervision(props: &Self::Props) -> Supervision {
        Supervision::one_for_one()
    }
}

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

#[derive(Clone, Copy, Debug)]
pub enum Limit {
    Amount(u32),
    Within { amount: u32, timespan: Duration },
}

pub trait IntoLimit {
    fn into_limit(self) -> Limit;
}

impl IntoLimit for Limit {
    fn into_limit(self) -> Limit {
        self
    }
}

impl IntoLimit for u32 {
    fn into_limit(self) -> Limit {
        Limit::Amount(self)
    }
}

impl IntoLimit for (u32, Duration) {
    fn into_limit(self) -> Limit {
        Limit::Within {
            amount: self.0,
            timespan: self.1,
        }
    }
}

impl IntoLimit for (Duration, u32) {
    fn into_limit(self) -> Limit {
        Limit::Within {
            amount: self.1,
            timespan: self.0,
        }
    }
}

pub struct Supervision {
    strategy: Strategy,
    directive: Directive,
    max_restarts: Option<Limit>,
    backoff: Option<Backoff>,
    deciders: Vec<Box<dyn Send + Sync + Fn(&Box<dyn Any + Send>) -> Option<Directive>>>,
}

impl Supervision {
    /// Initializes a one-for-one supervision strategy.
    ///
    /// Applies directives only to the failed process, maintaining
    /// the state of others with the same parent. The default directive is
    /// set to `Directive::Restart`, but can be overridden.
    ///
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Directive};
    ///
    /// Supervision::one_for_one()
    ///     .max_restarts(3)
    ///     .directive(Directive::Resume);
    /// ```
    pub fn one_for_one() -> Self {
        Self {
            strategy: Strategy::OneForOne {
                counter: Default::default(),
            },
            directive: Directive::Restart,
            max_restarts: None,
            backoff: None,
            deciders: vec![],
        }
    }

    /// Initializes a one-for-all supervision strategy.
    ///
    /// If any supervised process fails, all processes with the same parent
    /// process will have the same directive applied to them.
    /// The default directive is `Directive::Restart`, which can be overridden.
    ///
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Directive};
    ///
    /// Supervision::one_for_all()
    ///     .max_restarts(5)
    ///     .directive(Directive::Stop);
    /// ```
    pub fn one_for_all() -> Self {
        Self {
            strategy: Strategy::OneForAll {
                counter: Default::default(),
            },
            directive: Directive::Restart,
            max_restarts: None,
            backoff: None,
            deciders: vec![],
        }
    }

    /// Sets the directive to be applied when a child fails.
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Directive};
    ///
    /// let ignore_all_errors = Supervision::one_for_all().directive(Directive::Resume);
    /// let stop_all = Supervision::one_for_all().directive(Directive::Stop);
    /// let restart_all = Supervision::one_for_all().directive(Directive::Restart);
    /// ```
    pub fn directive(mut self, directive: Directive) -> Self {
        self.directive = directive;
        self
    }

    /// Specifies how many times a failed process will be
    /// restarted before giving up. By default there is no limit.
    /// If limit is reached, process is stopped.
    ///
    /// ## Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use speare::{Supervision, Directive, Limit};
    ///
    /// // Maximum of 3 restarts during the lifetime of this process.
    /// Supervision::one_for_one().max_restarts(3);
    ///
    /// Supervision::one_for_one().max_restarts(Limit::Amount(3));
    ///
    /// // Maximum of 3 restarts in a 1 seconds timespan.
    /// Supervision::one_for_one()
    ///     .max_restarts(Limit::Within { amount: 3, timespan: Duration::from_secs(1) });
    ///
    /// Supervision::one_for_one()
    ///     .max_restarts((3, Duration::from_secs(1)));
    /// ```
    pub fn max_restarts<T: IntoLimit>(mut self, max_restarts: T) -> Self {
        self.max_restarts = Some(max_restarts.into_limit());
        self
    }

    /// Configures the backoff strategy for restarts.
    ///
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Backoff};
    /// use std::time::Duration;
    ///
    /// // Using a static backoff duration
    /// Supervision::one_for_one()
    ///     .backoff(Backoff::Static(Duration::from_secs(5)));
    ///
    /// // Using an incremental backoff
    /// Supervision::one_for_one()
    ///     .backoff(Backoff::Incremental {
    ///         steps: Duration::from_secs(1),
    ///         max: Duration::from_secs(10),
    ///     });
    /// ```
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = Some(backoff);
        self
    }

    /// Allows specifying a closure that decides the directive for a failed process
    /// based on its error type. The closure is applied only if the error type
    /// successfully downcasts at runtime.
    ///
    /// ## Examples
    ///
    /// ```
    /// use async_trait::async_trait;
    /// use speare::{Ctx, Directive, Node, Process, Supervision};
    ///
    /// struct Parent;
    ///
    /// #[async_trait]
    /// impl Process for Parent {
    ///     type Props = ();
    ///     type Msg = ();
    ///     type Err = ();
    ///
    ///     async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///         ctx.spawn::<Foo>(());
    ///         ctx.spawn::<Bar>(());
    ///         Ok(Parent)
    ///     }
    ///
    ///     fn supervision(props: &Self::Props) -> Supervision {
    ///         Supervision::one_for_all()
    ///             .when(|e: &FooErr| Directive::Restart)
    ///             .when(|e: &BarErr| Directive::Stop)
    ///     }
    /// }
    ///
    /// struct Foo;
    ///
    /// struct FooErr(String);
    ///
    /// #[async_trait]
    /// impl Process for Foo {
    ///     type Props = ();
    ///     type Msg = ();
    ///     type Err = FooErr;
    ///
    ///     async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///         Err(FooErr("oh no".to_string()))
    ///     }
    /// }
    ///
    /// struct Bar;
    ///
    /// struct BarErr(String);
    ///
    /// #[async_trait]
    /// impl Process for Bar {
    ///     type Props = ();
    ///     type Msg = ();
    ///     type Err = BarErr;
    ///
    ///     async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///         Err(BarErr("oopsie".to_string()))
    ///     }
    /// }
    /// ```
    pub fn when<F, E>(mut self, f: F) -> Self
    where
        E: 'static,
        F: 'static + Send + Sync + Fn(&E) -> Directive,
    {
        let closure = move |any_e: &Box<dyn Any + Send>| {
            let e: &SharedErr<E> = any_e.downcast_ref()?;
            Some(f(e))
        };

        self.deciders.push(Box::new(closure));

        self
    }
}

type ProcessId = u64;

#[derive(Clone, Debug)]
struct RestartCount {
    count: u32,
    last_restart: Instant,
}

impl Default for RestartCount {
    fn default() -> Self {
        Self {
            count: Default::default(),
            last_restart: Instant::now(),
        }
    }
}

impl RestartCount {
    /// None means that max numbers of reset have been reached and we should not reset anymore
    fn get_backoff_duration(
        &mut self,
        max: Option<Limit>,
        backoff: Option<Backoff>,
    ) -> Option<Duration> {
        let (should_restart, new_count) = match max {
            None => (true, self.count + 1),

            Some(Limit::Amount(m)) => (self.count < m, self.count + 1),

            Some(Limit::Within { amount, timespan }) => {
                if self.last_restart + timespan > Instant::now() {
                    (self.count < amount, self.count + 1)
                } else {
                    (true, 1)
                }
            }
        };

        if should_restart {
            let dur = match backoff {
                None => Duration::ZERO,
                Some(Backoff::Static(dur)) => dur,
                Some(Backoff::Incremental { steps, max }) => cmp::min(steps * self.count, max),
            };

            self.count = new_count;
            self.last_restart = Instant::now();

            Some(dur)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
enum Strategy {
    OneForOne {
        counter: HashMap<ProcessId, RestartCount>,
    },

    OneForAll {
        counter: RestartCount,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum Directive {
    /// Resumes the process(es) as if nothing happened.
    Resume,
    /// Gracefully stops the process(es). If the process is currently handling a message, it will finish handling that and then immediately stop.
    Stop,
    /// Restarts the process(es), calling `Process::exit()` and subsequently `Process::init()`.
    Restart,
    /// Escalate the error to the parent `Process`.
    Escalate,
}

#[derive(Clone, Copy, Debug)]
pub enum Backoff {
    /// Uses the same backoff duration for every restart.
    Static(Duration),
    /// Uses a backoff that increases with each subsequent restart up to a maximum value.
    Incremental { steps: Duration, max: Duration },
}

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
    pub fn stop(&self) {
        let _ = self.proc_msg_tx.send(ProcMsg::FromHandle(ProcAction::Stop));
    }

    pub fn is_alive(&self) -> bool {
        !self.msg_tx.is_disconnected()
    }

    pub fn send<M: Into<Msg>>(&self, msg: M) {
        let _ = self.msg_tx.send(msg.into());
    }

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

    pub async fn req<Req, Res>(&self, req: Req) -> Result<Res, ReqErr>
    where
        Msg: From<Request<Req, Res>>,
    {
        let (req, res) = req_res(req);
        self.send(req);
        res.recv().await
    }

    pub async fn req_timeout<Req, Res>(&self, req: Req, timeout: Duration) -> Result<Res, ReqErr>
    where
        Msg: From<Request<Req, Res>>,
    {
        let (req, res) = req_res(req);
        self.send(req);
        res.recv_timeout(timeout).await
    }
}

pub struct Ctx<P>
where
    P: Process,
{
    id: u64,
    props: P::Props,
    handle: Handle<P::Msg>,
    msg_rx: Receiver<P::Msg>,
    parent_proc_msg_tx: Sender<ProcMsg>,
    proc_msg_rx: Receiver<ProcMsg>,
    children_proc_msg_tx: HashMap<u64, Sender<ProcMsg>>,
    supervision: Supervision,
    total_children: u64,
}

impl<P> Ctx<P>
where
    P: Process,
{
    /// Returns a reference to the `Process::Props` of the current `Process`.
    pub fn props(&self) -> &P::Props {
        &self.props
    }

    /// Returns a reference to a `Handle` of the current `Process`.
    pub fn this(&self) -> &Handle<P::Msg> {
        &self.handle
    }

    /// Clears all the messages from the mailbox.
    pub fn clear_mailbox(&self) {
        self.msg_rx.drain();
    }

    /// Spawns and supervises a child `Process`.
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Directive};
    /// // TODO!
    /// ```
    pub fn spawn<Child>(&mut self, props: Child::Props) -> Handle<Child::Msg>
    where
        Child: Process,
    {
        self.total_children += 1;
        let (msg_tx, msg_rx) = flume::unbounded(); // child
        let (proc_msg_tx, proc_msg_rx) = flume::unbounded(); // child

        let handle = Handle {
            msg_tx,
            proc_msg_tx,
        };

        let supervision = Child::supervision(&props);

        let ctx: Ctx<Child> = Ctx {
            id: self.total_children,
            props,
            handle: handle.clone(),
            msg_rx,
            parent_proc_msg_tx: self.handle.proc_msg_tx.clone(),
            proc_msg_rx,
            children_proc_msg_tx: Default::default(),
            total_children: 0,
            supervision,
        };

        spawn::<P, Child>(ctx, None);

        self.children_proc_msg_tx
            .insert(self.total_children, handle.proc_msg_tx.clone());

        handle
    }

    async fn handle_err(
        &mut self,
        e: Box<dyn Any + Send>,
        proc_id: u64,
        err_ack: Sender<()>,
    ) -> Option<()> {
        let directive = self
            .supervision
            .deciders
            .iter()
            .find_map(|f| f(&e))
            .unwrap_or(self.supervision.directive);

        match (&mut self.supervision.strategy, directive) {
            (Strategy::OneForOne { counter }, Directive::Restart) => {
                let child = self.children_proc_msg_tx.get(&proc_id)?;
                let child_restarts = counter.entry(proc_id).or_default();

                match child_restarts
                    .get_backoff_duration(self.supervision.max_restarts, self.supervision.backoff)
                {
                    Some(delay) => {
                        let _ = child.send(ProcMsg::FromParent(ProcAction::Restart(
                            Restart::without_ack(delay),
                        )));
                    }

                    None => {
                        // Stop child if max number of resets have been reached
                        let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                        self.children_proc_msg_tx.remove(&proc_id);
                    }
                }

                let _ = err_ack.send(());
            }

            (Strategy::OneForOne { counter }, Directive::Stop) => {
                let child = self.children_proc_msg_tx.get(&proc_id)?;
                let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                counter.remove(&proc_id);
                self.children_proc_msg_tx.remove(&proc_id);

                let _ = err_ack.send(());
            }

            (Strategy::OneForAll { counter }, Directive::Restart) => {
                match counter
                    .get_backoff_duration(self.supervision.max_restarts, self.supervision.backoff)
                {
                    Some(delay) => {
                        let _ = err_ack.send(());

                        let mut exit_ack_rxs = vec![];
                        let mut can_restart_txs = vec![];

                        for child in self.children_proc_msg_tx.values() {
                            let (restart, exit_ack_rx, can_restart_tx) = Restart::with_ack(delay);
                            exit_ack_rxs.push(exit_ack_rx);
                            can_restart_txs.push(can_restart_tx);

                            let _ = child.send(ProcMsg::FromParent(ProcAction::Restart(restart)));
                        }

                        for rx in exit_ack_rxs {
                            let _ = rx.recv_async().await;
                        }

                        for tx in can_restart_txs {
                            let _ = tx.send(());
                        }
                    }

                    None => {
                        for child in self.children_proc_msg_tx.values() {
                            let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                        }

                        self.children_proc_msg_tx.clear();
                    }
                }
            }

            (Strategy::OneForAll { .. }, Directive::Stop) => {
                let _ = err_ack.send(());

                for child in self.children_proc_msg_tx.values() {
                    let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                }

                self.children_proc_msg_tx.clear();
            }

            (_, Directive::Escalate) => {
                let (tx, _) = flume::unbounded();
                let _ = self.parent_proc_msg_tx.send(ProcMsg::FromChild {
                    child_id: self.id,
                    err: e,
                    ack: tx,
                });

                let _ = err_ack.send(());
            }

            (_, Directive::Resume) => {
                let _ = err_ack.send(());
            }
        };

        task::yield_now().await;

        None
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
enum ProcMsg {
    FromChild {
        child_id: u64,
        err: Box<dyn Any + Send>,
        ack: Sender<()>,
    },

    FromParent(ProcAction),
    FromHandle(ProcAction),
}

#[derive(Debug)]
struct Restart {
    delay: Duration,
    exit_ack_tx: Option<Sender<()>>,
    can_restart_rx: Option<Receiver<()>>,
}

impl Restart {
    fn with_ack(delay: Duration) -> (Self, Receiver<()>, Sender<()>) {
        let (exit_ack_tx, exit_ack_rx) = flume::unbounded();
        let (can_restart_tx, can_restart_rx) = flume::unbounded();

        let restart = Restart {
            delay,
            exit_ack_tx: Some(exit_ack_tx),
            can_restart_rx: Some(can_restart_rx),
        };

        (restart, exit_ack_rx, can_restart_tx)
    }

    fn without_ack(delay: Duration) -> Restart {
        Restart {
            delay,
            exit_ack_tx: None,
            can_restart_rx: None,
        }
    }

    /// Waits for signal from Parent to restart
    async fn sync(&self) {
        if let (Some(exit_ack_tx), Some(can_restart_rx)) = (&self.exit_ack_tx, &self.can_restart_rx)
        {
            let _ = exit_ack_tx.send(());
            let _ = can_restart_rx.recv_async().await;
        }
    }
}

#[derive(Debug)]
enum ProcAction {
    Restart(Restart),
    Stop,
}

fn spawn<Parent, Child>(mut ctx: Ctx<Child>, delay: Option<Duration>)
where
    Parent: Process,
    Child: Process,
{
    tokio::spawn(async move {
        if let Some(d) = delay.filter(|d| !d.is_zero()) {
            time::sleep(d).await;
        }

        let mut restart = None;

        match Child::init(&mut ctx).await {
            Err(e) => {
                let (tx, rx) = flume::unbounded();
                let _ = ctx.parent_proc_msg_tx.send(ProcMsg::FromChild {
                    child_id: ctx.id,
                    err: Box::new(SharedErr::new(e)),
                    ack: tx,
                });
                let _ = rx.recv_async().await;

                loop {
                    if let Ok(ProcMsg::FromParent(proc_action)) = ctx.proc_msg_rx.recv_async().await
                    {
                        if let ProcAction::Restart(r) = proc_action {
                            restart = Some(r);
                        }

                        break;
                    }
                }

                for child in ctx.children_proc_msg_tx.values() {
                    let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                }

                task::yield_now().await;

                if let Some(r) = restart {
                    r.sync().await;
                    spawn::<Parent, Child>(ctx, Some(r.delay))
                }
            }

            Ok(mut process) => {
                let mut exit_reason = None;

                loop {
                    tokio::select! {
                        biased;

                        proc_msg = ctx.proc_msg_rx.recv_async() => {
                            match proc_msg {
                                Err(_) => break,

                                Ok(ProcMsg::FromHandle(ProcAction::Stop) ) => {
                                    exit_reason = Some(ExitReason::Handle);
                                    break
                                },

                                Ok( ProcMsg::FromParent(ProcAction::Stop)) => {
                                    exit_reason = exit_reason.or(Some(ExitReason::Parent));
                                    break
                                },

                                Ok(ProcMsg::FromParent(ProcAction::Restart(r))) => {
                                    exit_reason = exit_reason.or(Some(ExitReason::Parent));
                                    restart = Some(r);
                                    break;
                                }

                                Ok(ProcMsg::FromChild { child_id, err, ack }) => {
                                    ctx.handle_err(err, child_id, ack).await;
                                }

                                Ok(_) => ()
                            }
                        }

                        recvd = ctx.msg_rx.recv_async() => {
                            match recvd {
                                Err(_) => break,

                                Ok(msg) => {
                                    if let Err(e) = process.handle(msg, &mut ctx).await {
                                        let e = SharedErr::new(e);
                                        exit_reason = Some(ExitReason::Err(e.clone()));
                                        let (tx, rx) = flume::unbounded();
                                        let _ = ctx.parent_proc_msg_tx.send(ProcMsg::FromChild {
                                            child_id: ctx.id,
                                            err: Box::new(e.clone()),
                                            ack: tx,
                                        });
                                        let _ = rx.recv_async().await;
                                    };
                                }
                            }
                        }
                    }
                }

                for child in ctx.children_proc_msg_tx.values() {
                    let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                }

                task::yield_now().await;

                let exit_reason = exit_reason.unwrap_or(ExitReason::Handle);
                process.exit(exit_reason, &mut ctx).await;

                if let Some(r) = restart {
                    r.sync().await;
                    spawn::<Parent, Child>(ctx, Some(r.delay))
                }
            }
        }
    });
}

#[derive(Default)]
pub struct Node {
    children_directive_tx: Vec<Sender<ProcMsg>>,
}

impl Drop for Node {
    fn drop(&mut self) {
        for directive_tx in &self.children_directive_tx {
            let _ = directive_tx.send(ProcMsg::FromHandle(ProcAction::Stop));
        }
    }
}

impl Node {
    pub fn spawn<P>(&mut self, props: P::Props) -> Handle<P::Msg>
    where
        P: Process,
    {
        let (msg_tx, msg_rx) = flume::unbounded();
        let (directive_tx, directive_rx) = flume::unbounded();
        let (ignore, _) = flume::unbounded();

        let handle = Handle {
            msg_tx,
            proc_msg_tx: directive_tx,
        };

        let supervision = P::supervision(&props);

        let mut ctx: Ctx<P> = Ctx {
            id: 0,
            total_children: 0,
            props,
            handle: handle.clone(),
            msg_rx,
            parent_proc_msg_tx: ignore,
            proc_msg_rx: directive_rx,
            children_proc_msg_tx: Default::default(),
            supervision,
        };

        tokio::spawn(async move {
            match P::init(&mut ctx).await {
                Err(_) => {
                    for child in ctx.children_proc_msg_tx.values() {
                        let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                    }

                    task::yield_now().await;
                }

                Ok(mut process) => {
                    let mut exit_reason = None;
                    loop {
                        tokio::select! {
                            biased;

                            proc_msg = ctx.proc_msg_rx.recv_async() => {
                                match proc_msg {
                                    Err(_) => break,

                                    Ok(ProcMsg::FromHandle(ProcAction::Stop) | ProcMsg::FromParent(ProcAction::Stop)) => {
                                        exit_reason = Some(ExitReason::Handle);
                                        break
                                    },

                                    Ok(ProcMsg::FromChild { child_id, err, ack }) => {
                                        ctx.handle_err(err, child_id, ack).await;
                                    }

                                    _ => {}
                                }
                            }

                            recvd = ctx.msg_rx.recv_async() => {
                                match recvd {
                                    Err(_) => break,

                                    Ok(msg) => {
                                        if let Err(e) = process.handle(msg, &mut ctx).await {
                                            exit_reason = Some(ExitReason::Err(SharedErr::new(e)));
                                            break;
                                        };
                                    }
                                }
                            }
                        }
                    }

                    for child in ctx.children_proc_msg_tx.values() {
                        let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                    }

                    task::yield_now().await;

                    let exit_reason = exit_reason.unwrap_or(ExitReason::Handle);
                    process.exit(exit_reason, &mut ctx).await;
                }
            }
        });

        self.children_directive_tx.push(handle.proc_msg_tx.clone());

        handle
    }
}
