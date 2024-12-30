use async_trait::async_trait;
use flume::{Receiver, Sender};
use futures::Stream;
use std::{any::Any, collections::HashMap, future::Future, time::Duration};
use tokio::{
    task,
    time::{self},
};

mod exit;
mod req_res;
mod stream;
mod supervision;

pub use exit::*;
pub use req_res::*;
pub use stream::*;
pub use supervision::*;

/// A thin abstraction over tokio tasks and flume channels, allowing for easy message passing
/// with a supervision tree to handle failures.
///
/// ## Example
/// ```
/// use speare::{Ctx, Process};
/// use async_trait::async_trait;
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
/// #[async_trait]
/// impl Process for Counter {
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
#[async_trait]
pub trait Process: Sized + Send + 'static {
    type Props: Send + 'static;
    type Msg: Send + 'static;
    type Err: Send + Sync + 'static;

    /// The constructor function that will be used to create an instance of your `Process`
    /// when spawning or restarting it.
    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err>;

    /// A function that will be called if your `Process` is stopped or restarted.
    async fn exit(&mut self, reason: ExitReason<Self>, ctx: &mut Ctx<Self>) {}

    /// Called everytime your `Process` receives a message.
    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        Ok(())
    }

    /// Allows to determine custom strategies for handling errors from child processes.
    fn supervision(props: &Self::Props) -> Supervision {
        Supervision::one_for_one()
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
    /// Stops the Process for which this `Handle<_>` is for.
    pub fn stop(&self) {
        let _ = self.proc_msg_tx.send(ProcMsg::FromHandle(ProcAction::Stop));
    }

    /// Returns true if the Process is still running, false if it has been stopped.
    pub fn is_alive(&self) -> bool {
        !self.msg_tx.is_disconnected()
    }

    /// Sends a message to the `Process` associated with this `Handle<_>`, failing silently if that process is no longer running.
    ///
    /// `send` can take advantage of `From<_>` implementations for the variants of the `Process::Msg` type.
    ///
    /// ## Example
    /// ```
    /// use speare::{Ctx, Node, Process};
    /// use async_trait::async_trait;
    /// use derive_more::From;
    /// use tokio::runtime::Runtime;
    ///
    /// Runtime::new().unwrap().block_on(async {
    ///     let mut node = Node::default();
    ///     let counter = node.spawn::<Counter>(());
    ///
    ///     // we can send a u32 directly because
    ///     // CounterMsg derives From
    ///     counter.send(10);
    /// });
    ///
    /// struct Counter(u32);
    ///
    /// #[derive(From)]
    /// enum CounterMsg {
    ///     Inc(u32),
    ///     Print,
    /// }
    ///
    /// #[async_trait]
    /// impl Process for Counter {
    ///     type Props = ();
    ///     type Msg = CounterMsg;
    ///     type Err = ();
    ///
    ///     async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///         Ok(Counter(0))
    ///     }
    ///
    ///     async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
    ///         match msg {
    ///             CounterMsg::Inc(x) => self.0 += x,
    ///             CounterMsg::Print => println!("Count is {}", self.0),
    ///         }
    ///
    ///         Ok(())
    ///     }
    /// }
    /// ```
    pub fn send<M: Into<Msg>>(&self, msg: M) {
        let _ = self.msg_tx.send(msg.into());
    }

    /// After the given duration, sends a message to the `Process` associated with this `Handle<_>`, failing silently if that process is no longer running.
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

    /// Sends a request to the `Process` as long as its messages implements `From<Request<Req,Res>>`.
    ///
    /// In `speare` a `Request<Req,Res>` allows a request-response transaction between processes.
    ///
    /// ## Example
    /// ```
    /// use speare::{req_res, Ctx, Node, Process, Request};
    /// use async_trait::async_trait;
    /// use derive_more::From;
    /// use tokio::runtime::Runtime;
    ///
    /// Runtime::new().unwrap().block_on(async {
    ///     let mut node = Node::default();
    ///     let parser = node.spawn::<Parser>(());
    ///
    ///     let num = parser.req("5".to_string()).await.unwrap();
    ///     assert_eq!(num, 5);
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
    /// impl Process for Parser {
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

    /// Sends a request to the `Process` as long as its messages implements `From<Request<Req,Res>>`.
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

/// The context surrounding the current `Process`.
///
/// Provides a collection of methods that allow you to:
/// - spawn other processes as children of the current process
/// - access the `Handle<_>` for the currrent process
/// - access this process's props
/// - clear this process's mailbox
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

    pub fn stream<F, Fut, S, T, E>(&mut self, f: F) -> StreamBuilder<'_, P, F, Fut, S, T, E, NoSink>
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = S> + Send + 'static,
        S: Stream<Item = Result<T, E>> + Send + 'static + Unpin,
        T: Send + 'static,
        E: Send + Sync + 'static,
    {
        StreamBuilder::new(f, self)
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

enum NodeProcMsg {
    SpawnedChild(Sender<ProcMsg>),
    Stop,
}

fn node_proc() -> Sender<NodeProcMsg> {
    let (tx, rx) = flume::unbounded();

    task::spawn(async move {
        let mut children = Vec::new();

        while let Ok(msg) = rx.recv_async().await {
            match msg {
                NodeProcMsg::SpawnedChild(child) => {
                    children.push(child);
                }

                NodeProcMsg::Stop => {
                    for child in &children {
                        let _ = child.send(ProcMsg::FromHandle(ProcAction::Stop));
                    }

                    break;
                }
            }
        }
    });

    tx
}

/// A `Node` owns a collection of unsupervised top-level processes.
/// If the `Node` is dropped, all of its processes are stopped.
///
/// ### Unsupervised Processes
/// Unsupervised processes will be stopped when they error. Since they are unsupervised,
/// the errors won't be handled and they will not be automatically restarted.
pub struct Node {
    proc: Sender<NodeProcMsg>,
}

impl Default for Node {
    fn default() -> Self {
        Self { proc: node_proc() }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.proc.send(NodeProcMsg::Stop);
    }
}

impl Node {
    /// Spawns an unsupervised process.
    pub fn spawn<P>(&self, props: P::Props) -> Handle<P::Msg>
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

        let _ = self
            .proc
            .send(NodeProcMsg::SpawnedChild(handle.proc_msg_tx.clone()));

        handle
    }
}
