use std::{
    any::{Any, TypeId},
    collections::HashMap,
    future::Future,
    ops::Deref,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use flume::{Sender, TrySendError};
use tokio::{
    sync::Mutex,
    task::{self},
    time,
};

use crate::process::{AskErr, ExitReason, ExitSignal, Handler, Pid, Process};

type Unknown = Box<dyn Any + Sync + Send + 'static>;
type AnyProc = Unknown;
type AnyCtx = Unknown;
type AnyMsg = Box<dyn Any + Sync + Send + 'static>;

type ProcessRunner = Arc<
    dyn Fn(AnyProc, AnyCtx, AnyMsg) -> Pin<Box<dyn Future<Output = (AnyProc, AnyCtx)> + Send>>
        + Send
        + Sync
        + 'static,
>;

type ExitSignalSender = Box<
    dyn FnOnce(Unknown) -> Pin<Box<dyn Future<Output = Unknown> + Send>> + Send + Sync + 'static,
>;

pub type MessageSender = Sender<(ProcessRunner, AnyMsg)>;
type Subscribers = HashMap<TypeId, Vec<(ProcessRunner, MessageSender)>>;

pub enum ExitMessage {
    Exit(Unknown),
    StoreMonitor(ExitSignalSender),
}

/// A `Node` provides methods to spawn, send messages to, and terminate processes.
/// It is essentially the environment in which a `Process` is spawned. You can create
/// multiple nodes in your application to isolate processes from each other, but in
/// most cases you'll need only one throughout your whole program.
///
/// ## Example
/// ```ignore
/// async fn example() {
///     let node = Node::default();
///     node.spawn(MyProcess).await;
/// }
/// ```
/// Learn more on [The Speare Book](https://vmenge.github.io/speare/spawning_a_process.html)
#[derive(Default, Clone)]
pub struct Node {
    count: Arc<Mutex<u32>>,
    subscribers: Arc<Mutex<Subscribers>>,
}

impl Node {
    /// Spawns a `Process`.
    /// ## Example
    /// ```ignore
    /// async fn example() {
    ///     let node = Node::default();
    ///     let my_proc_pid = node.spawn(MyProcess).await;
    /// }
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/spawning_a_process.html)
    pub async fn spawn<P>(&self, proc: P) -> Pid<P>
    where
        P: Process + Send + Sync + 'static,
    {
        let mut count = self.count.lock().await;
        let pid = self.spawn_proc(*count, proc).await;
        *count += 1;

        pid
    }

    /// Terminates a `Process`. `ExitSignal<P>` will be received after the current
    /// (if any) message is finished being handled by the `Process`.
    /// ## Example
    /// ```ignore
    /// let node = Node::default();
    ///let counter_pid = node.spawn(Counter::default()).await;
    ///node.exit(&counter_pid, ExitReason::Shutdown).await;
    /// ````
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/lifecycle_management.html)
    pub async fn exit<P>(&self, pid: &Pid<P>, reason: ExitReason<P>)
    where
        P: Process + Send + Sync + 'static,
    {
        let exit_signal = ExitSignal::new(pid.clone(), reason);
        let exit_signal = ExitMessage::Exit(Box::new(exit_signal));

        pid.exit_tx.try_send(exit_signal).ok();
    }

    /// Publishes message to any `Process` that implements a `Handler` for it and that has explicitly subscribed to it.
    ///
    /// **Note:** Messsage must implement `Clone`, as it will be called to send the message to multiple processes.
    /// ## Example
    /// ```ignore
    /// struct Cat;
    ///
    ///#[process]
    ///impl Cat {
    ///    #[subscriptions]
    ///    async fn subs(&self, evt: &EventBus<Self>) {
    ///        evt.subscribe::<SayHi>().await;
    ///    }
    ///
    ///    #[handler]
    ///    async fn hi(&mut self, msg: SayHi) -> Reply<(), ()> {
    ///        println!("MEOW!");
    ///        reply(())
    ///    }
    ///}
    ///
    ///struct Dog;
    ///
    ///#[tokio::main]
    ///async fn main() {
    ///    let node = Node::default();
    ///    node.spawn(Cat).await;
    ///    node.publish(SayHi).await;  
    ///    // "MEOW!"
    ///}
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/pub_sub.html)
    pub async fn publish<M>(&self, msg: M)
    where
        M: 'static + Send + Sync + Clone,
    {
        let mut subscribers = self.subscribers.lock().await;
        if let Some(subs) = subscribers.get_mut(&TypeId::of::<M>()) {
            let msg = Box::new(msg);
            let mut to_delete: Option<Vec<usize>> = None;

            for (idx, (handler, runner_tx)) in subs.iter().enumerate() {
                let res = runner_tx.try_send((handler.clone(), msg.clone()));

                // cleanup txs to dropped rxs
                // TODO: move logic to separate place
                if let Err(TrySendError::Disconnected(_)) = res {
                    if let Some(to_del) = to_delete.as_mut() {
                        to_del.push(idx);
                    } else {
                        to_delete = Some(vec![idx]);
                    }
                }
            }

            if let Some(to_delete) = to_delete.as_mut() {
                to_delete.sort_by(|a, b| b.cmp(a));

                for i in to_delete {
                    subs.remove(*i);
                }
            }
        }
    }

    /// Returns `true` if the `Process` for the given `Pid<P>` is still running.
    /// ## Example
    /// ```ignore
    /// async fn exit_kills_process_gracefully() {
    ///    let node = Node::default();
    ///    let pid = node.spawn(MyProc).await;
    ///
    ///    let is_alive_before_exit = node.is_alive(&pid);
    ///    node.exit(&pid, ExitReason::Shutdown).await;
    ///    // we wait 0ms here just to be sure we won't check again before the Process finishes shutting down
    ///    tokio::time::sleep(Duration::from_millis(0)).await;
    ///    let is_alive_after_exit = node.is_alive(&pid);
    ///
    ///    assert!(is_alive_before_exit);
    ///    assert!(!is_alive_after_exit);
    ///}
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/lifecycle_management.html)
    pub fn is_alive<P>(&self, pid: &Pid<P>) -> bool
    where
        P: 'static + Send + Sync + Process,
    {
        !pid.runner_tx.is_disconnected()
    }

    /// Sends a message to a `Process` without waiting for it to be handled.
    /// ## Example
    /// ```ignore
    ///async fn example() {
    ///    let node = Node::default();
    ///    let dog_pid = node.spawn(Dog::new()).await;
    ///
    ///    // Fire and forget
    ///    node.tell(&dog_pid, Bark).await;
    ///}
    ///
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/handlers.html)
    pub async fn tell<P, M>(&self, pid: &Pid<P>, msg: M)
    where
        P: 'static + Send + Sync + Process + Handler<M>,
        M: 'static + Send + Sync,
    {
        let handler = message_handler::<P, M>(None);
        let msg = Box::new(msg);
        pid.runner_tx.try_send((handler, msg)).ok();
    }

    /// Sends a message to a `Process` after the specified `Duration`.
    /// ## Example
    /// ```ignore
    ///async fn example() {
    ///    let node = Node::default();
    ///    let dog_pid = node.spawn(Dog::new()).await;
    ///
    ///    // Fire and forget after 10 milliseconds
    ///    node.tell_in(&dog_pid, GiveBone, Duration::from_millis(10)).await;
    ///}
    ///
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/handlers.html)
    pub async fn tell_in<P, M>(&self, pid: &Pid<P>, msg: M, delay: Duration)
    where
        P: 'static + Send + Sync + Process + Handler<M>,
        M: 'static + Send + Sync,
    {
        let node = self.clone();
        let pid = pid.clone();
        task::spawn(async move {
            time::sleep(delay).await;
            node.tell(&pid, msg).await;
        });
    }

    /// Sends a message to a `Process`, waiting for it to respond.
    /// ## Example
    /// ```ignore
    ///async fn example() {
    ///    let node = Node::default();
    ///    let dog_pid = node.spawn(Dog::new()).await;
    ///
    ///    // Request response
    ///    let greeting = node.ask(&dog_pid, SayHi).await.unwrap_or_else(|_| "".to_string());
    ///    println!("The dog says: {}", greeting);
    ///}
    ///
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/handlers.html)
    pub async fn ask<P, M>(
        // TODO: add default timeout, and make it configureable
        &self,
        pid: &Pid<P>,
        msg: M,
    ) -> Result<<P as Handler<M>>::Ok, AskErr<P, M>>
    where
        P: 'static + Send + Sync + Process + Handler<M>,
        M: 'static + Send + Sync,
    {
        let (tx, rx) = flume::bounded(1);
        let handler = message_handler::<P, M>(Some(tx));
        let msg = Box::new(msg);

        if pid.runner_tx.send_async((handler, msg)).await.is_err() {
            return Err(AskErr::Exited);
        }

        match rx.recv_async().await {
            Err(_) => Err(AskErr::NoReply),
            Ok(Err(e)) => Err(AskErr::Handler(e)),
            Ok(Ok(Some(v))) => Ok(v),
            Ok(Ok(None)) => unreachable!(),
        }
    }

    async fn spawn_proc<P>(&self, id: u32, mut process: P) -> Pid<P>
    where
        P: Process + Send + Sync + 'static,
    {
        let (msg_tx, msg_rx): (MessageSender, _) = flume::unbounded();
        let (exit_tx, exit_rx) = flume::unbounded();
        let pid: Pid<P> = Pid::new(id, msg_tx, exit_tx);

        let evt = EventBus::new(self.clone(), pid.clone());
        process.subscriptions(&evt).await;

        let ctx = Ctx::new(evt.node, evt.pid);

        task::spawn(async move {
            process.on_init(&ctx).await;
            let mut ctx = Box::new(ctx) as AnyCtx;
            let mut process: Unknown = Box::new(process);
            let mut monitors: Vec<ExitSignalSender> = vec![];

            loop {
                tokio::select! {
                    biased;

                    exit_msg = exit_rx.recv_async() => {
                        match exit_msg {
                            Err(_) => (),

                            Ok(ExitMessage::StoreMonitor(exit_signal_sender)) => {
                                monitors.push(exit_signal_sender);
                            }

                            Ok(ExitMessage::Exit(mut any_signal)) => {
                                let process = process.downcast_mut::<P>().unwrap();
                                let ctx = ctx.downcast::<Ctx<P>>().unwrap();
                                process.on_exit(&ctx).await;

                                for monitor in monitors {
                                    any_signal = monitor(any_signal).await;
                                }

                                break;
                            }
                        }
                    }

                    handler = msg_rx.recv_async() => {
                        if let Ok((handler, msg)) = handler {
                            (process, ctx) = handler(process, ctx, msg).await;
                        }
                    }
                }
            }
        });

        pid
    }
}

type Response<P, M> = Result<Option<<P as Handler<M>>::Ok>, <P as Handler<M>>::Err>;

fn message_handler<P, M>(responder: Option<Sender<Response<P, M>>>) -> ProcessRunner
where
    P: 'static + Send + Sync + Process + Handler<M>,
    M: 'static + Send + Sync,
{
    Arc::new(move |actor, ctx, msg| {
        let mut proc = actor.downcast::<P>().unwrap();
        let mut ctx = ctx.downcast::<Ctx<P>>().unwrap();
        let msg = msg.downcast::<M>().unwrap();
        let responder = responder.clone();

        Box::pin(async move {
            if let Some(responder) = &responder {
                ctx.responder = Some(Box::new(responder.clone()));
            }

            let res = proc.handle(*msg, &ctx).await;

            if let Some(responder) = &responder {
                match res {
                    Ok(None) => (),

                    _ => {
                        responder.send(res).ok();
                    }
                }
            }

            (proc as Unknown, ctx as Unknown)
        })
    })
}

/// Allows you to access methods in the `Node` this `Process` belongs to, while
/// providing extra functionality such as this processes' id, `Process` monitoring,
/// and deferred replies.
/// Learn more on [The Speare Book](https://vmenge.github.io/speare)
pub struct Ctx<P>
where
    P: Send + Sync + 'static,
{
    node: Node,
    pid: Pid<P>,
    pub(crate) responder: Option<Unknown>,
}

impl<P> Deref for Ctx<P>
where
    P: Send + Sync + 'static,
{
    type Target = Node;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl<P> Ctx<P>
where
    P: Send + Sync + 'static,
{
    fn new(node: Node, pid: Pid<P>) -> Self {
        Self {
            node,
            pid,
            responder: None,
        }
    }

    /// The `Pid` of the current `Process`
    pub fn this(&self) -> &Pid<P> {
        &self.pid
    }
}

impl<P> Ctx<P>
where
    P: 'static + Send + Sync,
{
    /// Allows the current `Process` to receive a message when the `Process` with the id given in the arguments
    /// exits. Can only be called if the current `Process` implementas handler for the `ExitSignal<P>` of the monitored
    /// `Process`.
    /// ## Example
    /// ```ignore
    /// struct ProcA;
    ///
    ///#[process(Error = String)]
    ///impl ProcA {}
    ///
    ///struct ProcB {
    ///    a_pid: Pid<ProcA>
    ///}
    ///
    ///#[process]
    ///impl ProcB {
    ///    #[on_init]
    ///    async fn init(&mut self, ctx: &Ctx<Self>) {
    ///        ctx.monitor(&self.a_pid);
    ///        ctx.exit(&a_pid, ExitReason::Err("something went wrong".to_string())).await;
    ///    }
    ///
    ///    #[handler]
    ///    async fn handle_proc_a_exit(&mut self, signal: ExitSignal<ProcA>) -> Reply<(), ()> {
    ///        let reason = match signal.reason() {
    ///            ExitReason::Normal => "finishing running its tasks",
    ///            ExitReason::Shutdown => "intentional interrupt by another process",
    ///            ExitReason::Err(e) => e,
    ///        };
    ///
    ///        println!("ProcA exited due to: {}", reason);
    ///
    ///        reply(())
    ///    }
    ///}
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/lifecycle_management.html)
    pub fn monitor<Proc>(&self, pid: &Pid<Proc>)
    where
        Proc: Sync + Send + Process + 'static,
        P: 'static + Send + Sync + Process + Handler<ExitSignal<Proc>>,
    {
        let node = self.deref().clone();
        let this = self.this().clone();

        let exit_signal_sender: ExitSignalSender = Box::new(|any_exit_signal: Unknown| {
            Box::pin(async move {
                let exit_signal = any_exit_signal.downcast::<ExitSignal<Proc>>().unwrap();
                node.tell(&this, exit_signal.deref().clone()).await;
                let any_exit_signal: Unknown = exit_signal;
                any_exit_signal
            })
        });

        pid.exit_tx
            .send(ExitMessage::StoreMonitor(exit_signal_sender))
            .ok();
    }
}

impl<P> Ctx<P>
where
    P: 'static + Send + Sync,
{
    /// Creates a `Responder`, allowing you to respond to the current message at a later time.
    /// Will return `None` if the current `Handler` is called with `.tell` instead of `.ask`.
    /// ## Example
    /// ```ignore
    /// use speare::*;
    ///
    ///struct SayHi;
    ///struct GiveBone;
    ///
    ///#[derive(Default)]
    ///struct Dog {
    ///    hi_responder: Option<Responder<Self, SayHi>>,
    ///}
    ///
    ///#[process]
    ///impl Dog {
    ///    // the hi Handler specifies that it returns a String as a response,
    ///    // thus when we the Responder on get_bone, the Reply sent through
    ///    // it must be a String.
    ///    #[handler]
    ///    async fn hi(&mut self, _msg: SayHi, ctx: &Ctx<Self>) -> Reply<String, ()> {
    ///        self.hi_responder = ctx.responder::<SayHi>();
    ///
    ///        noreply()
    ///    }
    ///
    ///    #[handler]
    ///    async fn get_bone(&mut self, _msg: GiveBone) -> Reply<(), ()> {
    ///        if let Some(responder) = &self.hi_responder {
    ///            responder.reply(Ok("Hello".to_string()))
    ///        }
    ///
    ///        reply(())
    ///    }
    ///}
    /// ```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/reply.html#deferring-replies)
    pub fn responder<M>(&self) -> Option<Responder<P, M>>
    where
        P: 'static + Send + Sync + Handler<M>,
        M: 'static + Send + Sync,
    {
        let responder_as_any = self.responder.as_ref()?.as_ref();
        let responder = responder_as_any.downcast_ref::<Sender<Response<P, M>>>()?;

        Some(Responder {
            responder: responder.clone(),
        })
    }
}

pub struct Responder<P, M>
where
    P: 'static + Send + Sync + Process + Handler<M>,
    M: 'static + Send + Sync,
{
    responder: Sender<Response<P, M>>,
}

impl<P, M> Responder<P, M>
where
    P: 'static + Send + Sync + Handler<M>,
    M: 'static + Send + Sync,
{
    pub fn reply(&self, msg: Result<P::Ok, P::Err>) {
        let msg = msg.map(Some);
        self.responder.send(msg).ok();
    }
}

/// Allows the current `Process` to subscribe to global publishes of specific messages.
pub struct EventBus<P>
where
    P: Send + Sync + 'static,
{
    node: Node,
    pid: Pid<P>,
}

impl<P> EventBus<P>
where
    P: Send + Sync + 'static,
{
    fn new(node: Node, pid: Pid<P>) -> Self {
        Self { node, pid }
    }

    /// Subscribes an existing `Handler<M>` implementation to receive `Node`-wide
    /// publishes of message `M`.
    /// The current `Process` **must** have a `Handler` implemented for any message it wishes to subscribe to.
    /// ## Example
    /// ```ignore
    ///struct Cat;
    ///
    ///#[process]
    ///impl Cat {
    ///    #[subscriptions]
    ///    async fn subs(&self, evt: &EventBus<Self>) {
    ///        evt.subscribe::<SayHi>().await;
    ///    }
    ///
    ///    #[handler]
    ///    async fn hi(&mut self, msg: SayHi) -> Reply<(), ()> {
    ///        println!("MEOW!");
    ///        reply(())
    ///    }
    ///}
    ///```
    /// Learn more on [The Speare Book](https://vmenge.github.io/speare/pub_sub.html)
    pub async fn subscribe<M>(&self)
    where
        P: Handler<M> + Send + Sync + 'static,
        M: 'static + Send + Sync + Clone,
    {
        let mut subscribers = self.node.subscribers.lock().await;
        let entry = subscribers
            .entry(TypeId::of::<M>())
            .or_insert_with(Vec::new);

        let proc_runner = message_handler::<P, M>(None);
        entry.push((proc_runner, self.pid.runner_tx.clone()));
    }
}
