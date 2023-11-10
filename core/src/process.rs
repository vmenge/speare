use crate::node::{Ctx, EventBus, ExitMessage, MessageSender};
use async_trait::async_trait;
use flume::Sender;
use std::marker::PhantomData;

#[derive(Debug)]
pub struct Pid<P>
where
    P: Process,
{
    pub(crate) id: u32,
    pub(crate) runner_tx: MessageSender,
    pub(crate) exit_tx: Sender<ExitMessage>,
    phantom: PhantomData<P>,
}

impl<P> PartialEq for Pid<P>
where
    P: Process,
{
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<P> Clone for Pid<P>
where
    P: Process,
{
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            runner_tx: self.runner_tx.clone(),
            exit_tx: self.exit_tx.clone(),
            phantom: PhantomData,
        }
    }
}

impl<P> Pid<P>
where
    P: Process,
{
    pub(crate) fn new(id: u32, runner_tx: MessageSender, exit_tx: Sender<ExitMessage>) -> Self {
        Pid {
            id,
            runner_tx,
            exit_tx,
            phantom: PhantomData,
        }
    }
}

#[allow(unused_variables)]
#[async_trait]
pub trait Process: Sized + Sync + Send {
    type Error: Clone + Sync + Send;

    async fn subscriptions(&self, evt: &EventBus<Self>) {}
    async fn on_init(&mut self, ctx: &Ctx<Self>) {}
    async fn on_exit(&mut self, ctx: &Ctx<Self>) {}
}

#[async_trait]
pub trait Handler<Message>: Process + Sized + Send + Sync
where
    Message: 'static + Send + Sync,
{
    type Ok: Send + Sync;
    type Err: Send + Sync;

    async fn handle(
        &mut self,
        msg: Message,
        ctx: &Ctx<Self>,
    ) -> Result<Option<Self::Ok>, Self::Err>;
}

#[derive(Debug)]
pub enum AskErr<P, M>
where
    P: Process + Handler<M>,
    M: 'static + Send + Sync,
{
    Exited,
    NoReply,
    Handler(P::Err),
}

pub fn reply<T, E>(item: T) -> Result<Option<T>, E> {
    Ok(Some(item))
}

pub fn noreply<T, E>() -> Result<Option<T>, E> {
    Ok(None)
}

pub type Reply<T, E> = core::result::Result<Option<T>, E>;

pub enum ExitReason<P>
where
    P: Process,
{
    /// When a `Process` comes to the end of its execution without any errors.
    Normal,
    /// An intentional stop, interrupting the `Process`.
    Shutdown,
    /// `Process` exited with Error.
    Err(P::Error),
}

impl<P> Clone for ExitReason<P>
where
    P: Process,
{
    fn clone(&self) -> Self {
        match self {
            ExitReason::Normal => ExitReason::Normal,
            ExitReason::Shutdown => ExitReason::Shutdown,
            ExitReason::Err(e) => ExitReason::Err(e.clone()),
        }
    }
}

pub struct ExitSignal<P>
where
    P: Process,
{
    pid: Pid<P>,
    reason: ExitReason<P>,
}

impl<P> Clone for ExitSignal<P>
where
    P: Process,
{
    fn clone(&self) -> Self {
        Self {
            pid: self.pid.clone(),
            reason: self.reason.clone(),
        }
    }
}

impl<P> ExitSignal<P>
where
    P: Process,
{
    pub fn new(pid: Pid<P>, reason: ExitReason<P>) -> Self {
        Self { pid, reason }
    }

    pub fn pid(&self) -> &Pid<P> {
        &self.pid
    }

    pub fn reason(&self) -> &ExitReason<P> {
        &self.reason
    }
}
