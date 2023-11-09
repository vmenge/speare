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

use crate::process::{AskErr, Handler, Pid, Process};

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

pub type MessageSender = Sender<(ProcessRunner, AnyMsg)>;
type Subscribers = HashMap<TypeId, Vec<(ProcessRunner, MessageSender)>>;

#[derive(Default, Clone)]
pub struct Node {
    count: Arc<Mutex<u32>>,
    subscribers: Arc<Mutex<Subscribers>>,
}

impl Node {
    /// Spawns a `Process`.
    pub async fn spawn<P>(&self, proc: P) -> Pid<P>
    where
        P: Process + Send + Sync + 'static,
    {
        let mut count = self.count.lock().await;
        let pid = self.spawn_proc(*count, proc).await;
        *count += 1;

        pid
    }

    /// Terminates a `Process`. Exit signal will be received after the current
    /// (if any) message currently being handled by the `Process`.
    pub async fn exit<P>(&self, pid: &Pid<P>)
    where
        P: Process + Send + Sync + 'static,
    {
        pid.exit_tx.try_send(true).ok();
    }

    /// Publishes message to any `Process` that implements a `Handler` for it and that has subscribed to it.
    ///
    /// **Note:** Messsage must implement `Clone`, as it will be called to send the message to multiple processes.
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
    pub fn is_alive<P>(&self, pid: &Pid<P>) -> bool
    where
        P: 'static + Send + Sync + Process,
    {
        !pid.runner_tx.is_disconnected()
    }

    /// Sends a message to a `Process` without waiting for it to be handled.
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

    /// Sends a message to a `Process`, waiting for it to be handled and for its respective response.
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

            loop {
                tokio::select! {
                    biased;

                    _ = exit_rx.recv_async() => {
                        let process = process.downcast_mut::<P>().unwrap();
                        let ctx = ctx.downcast::<Ctx<P>>().unwrap();
                        process.on_exit(&ctx).await;
                        break;
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

pub struct Ctx<P>
where
    P: Process + Send + Sync + 'static,
{
    node: Node,
    pid: Pid<P>,
    pub(crate) responder: Option<Unknown>,
}

impl<P> Deref for Ctx<P>
where
    P: Process + Send + Sync + 'static,
{
    type Target = Node;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl<P> Ctx<P>
where
    P: Process + Send + Sync + 'static,
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
    P: 'static + Send + Sync + Process,
{
    pub fn responder<M>(&self) -> Option<Responder<P, M>>
    where
        P: 'static + Send + Sync + Process + Handler<M>,
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
    P: 'static + Send + Sync + Process + Handler<M>,
    M: 'static + Send + Sync,
{
    pub fn reply(&self, msg: Result<P::Ok, P::Err>) {
        let msg = msg.map(Some);
        self.responder.send(msg).ok();
    }
}

pub struct EventBus<P>
where
    P: Process + Send + Sync + 'static,
{
    node: Node,
    pid: Pid<P>,
}

impl<P> EventBus<P>
where
    P: Process + Send + Sync + 'static,
{
    fn new(node: Node, pid: Pid<P>) -> Self {
        Self { node, pid }
    }

    /// Subscribes an existing `Handler<M>` implementation to receive `Node`-wide
    /// publishes of message `M`.
    pub async fn subscribe<M>(&self)
    where
        P: Process + Handler<M> + Send + Sync + 'static,
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
