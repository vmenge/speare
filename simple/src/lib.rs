use async_trait::async_trait;
use flume::{Receiver, Sender};
use std::{any::Any, cmp, collections::HashMap, time::Duration};
use tokio::time;

// questions
// when a parent process restarts, should it also stop its children? (yes?)

pub struct Request<Req, Res> {
    data: Req,
    tx: Sender<Res>,
}

impl<Req, Res> Request<Req, Res> {
    pub fn data(&self) -> &Req {
        &self.data
    }

    pub fn reply(&self, res: Res) {
        let _ = self.tx.send(res);
    }
}

pub struct Response<Res> {
    rx: Receiver<Res>,
}

#[derive(Debug)]
pub enum AskErr {
    Failure,
    Timeout,
}

impl<Res> Response<Res> {
    pub async fn recv(self) -> Result<Res, AskErr> {
        self.rx.recv_async().await.map_err(|_| AskErr::Failure)
    }

    pub async fn recv_timeout(self, dur: Duration) -> Result<Res, AskErr> {
        time::timeout(dur, self.recv())
            .await
            .map_err(|_| AskErr::Timeout)
            .and_then(|x| x)
    }
}

pub fn ask<Req, Res>(req: Req) -> (Request<Req, Res>, Response<Res>) {
    let (tx, rx) = flume::unbounded();
    (Request { data: req, tx }, Response { rx })
}

#[allow(unused_variables)]
#[async_trait]
pub trait Process: Sized + Send + 'static {
    type Props: Send + Sync + 'static;
    type Msg: Send + 'static;
    type Err: Send + 'static;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err>;

    async fn exit(&mut self, ctx: &mut Ctx<Self>) {}

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        Ok(())
    }

    fn supervision() -> Supervision {
        Supervision::one_for_one()
    }
}

pub struct Supervision {
    strategy: Strategy,
    directive: Directive,
    max_restarts: Option<u32>,
    backoff: Option<Backoff>,
    deciders: Vec<Box<dyn Send + Sync + Fn(&Box<dyn Any + Send>) -> Option<Directive>>>,
}

impl Supervision {
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

    pub fn one_for_all() -> Self {
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

    pub fn directive(mut self, directive: Directive) -> Self {
        self.directive = directive;
        self
    }

    pub fn max_restarts(mut self, max_restarts: u32) -> Self {
        self.max_restarts = Some(max_restarts);
        self
    }

    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = Some(backoff);
        self
    }

    pub fn when<F, E>(mut self, f: F) -> Self
    where
        E: 'static,
        F: 'static + Send + Sync + Fn(&E) -> Directive,
    {
        let closure = move |any_e: &Box<dyn Any + Send>| {
            let e: &E = any_e.downcast_ref()?;
            Some(f(e))
        };

        self.deciders.push(Box::new(closure));

        self
    }
}

type ProcessId = u64;
type RestartCount = u32;

#[derive(Clone, Debug)]
pub enum Strategy {
    OneForOne {
        counter: HashMap<ProcessId, RestartCount>,
    },

    OneForAll {
        counter: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum Directive {
    /// Resumes the process that had the error as if nothing happened.
    Resume,
    /// Gracefully stops the process that had the error. If the process is currently handling a message, it will finish handling that and then immediately stop.
    Stop,
    /// Restarts the process that had the error, calling its `on_exit`, `on_restart` and `on_init` methods.
    Restart,
}

#[derive(Clone, Copy, Debug)]
pub enum Backoff {
    Static(Duration),
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
    prox_msg_rx: Receiver<ProcMsg>,
    children_proc_msg_tx: HashMap<u64, Sender<ProcMsg>>,
    supervision: Supervision,
    total_children: u64,
}

impl<P> Ctx<P>
where
    P: Process,
{
    pub fn props(&self) -> &P::Props {
        &self.props
    }

    pub fn this(&self) -> &Handle<P::Msg> {
        &self.handle
    }

    pub fn spawn<Child>(&mut self, props: Child::Props) -> Handle<Child::Msg>
    where
        Child: Process,
    {
        self.total_children += 1;
        let (msg_tx, msg_rx) = flume::unbounded(); // child
        let (directive_tx, directive_rx) = flume::unbounded(); // child

        let handle = Handle {
            msg_tx,
            proc_msg_tx: directive_tx,
        };

        let ctx: Ctx<Child> = Ctx {
            id: self.total_children,
            props,
            handle: handle.clone(),
            msg_rx,
            parent_proc_msg_tx: self.handle.proc_msg_tx.clone(),
            prox_msg_rx: directive_rx,
            children_proc_msg_tx: Default::default(),
            total_children: 0,
            supervision: Child::supervision(),
        };

        spawn::<P, Child>(ctx, None);

        self.children_proc_msg_tx
            .insert(self.total_children, handle.proc_msg_tx.clone());

        handle
    }

    async fn handle_err(&mut self, e: Box<dyn Any + Send>, id: u64) -> Option<()> {
        let directive = self
            .supervision
            .deciders
            .iter()
            .find_map(|f| f(&e))
            .unwrap_or(self.supervision.directive);

        match (&mut self.supervision.strategy, directive) {
            (Strategy::OneForOne { counter }, Directive::Restart) => {
                let child = self.children_proc_msg_tx.get(&id)?;
                let child_restarts = counter.entry(id).or_default();

                let delay = calc_backoff(
                    self.supervision.max_restarts,
                    self.supervision.backoff,
                    *child_restarts,
                )?;

                *child_restarts += 1;

                let _ = child.send(ProcMsg::FromParent(ProcAction::RestartIn(delay)));
            }

            (Strategy::OneForOne { counter }, Directive::Stop) => {
                let child = self.children_proc_msg_tx.get(&id)?;
                let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                counter.remove(&id);
                self.children_proc_msg_tx.remove(&id);
            }

            (Strategy::OneForAll { counter }, Directive::Restart) => {
                let delay = calc_backoff(
                    self.supervision.max_restarts,
                    self.supervision.backoff,
                    *counter,
                )?;

                *counter += 1;

                for child in self.children_proc_msg_tx.values() {
                    let _ = child.send(ProcMsg::FromParent(ProcAction::RestartIn(delay)));
                }
            }

            (Strategy::OneForAll { .. }, Directive::Stop) => {
                for child in self.children_proc_msg_tx.values() {
                    let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                }

                self.children_proc_msg_tx.clear();
            }

            (_, Directive::Resume) => (),
        };

        time::sleep(Duration::ZERO).await;

        None
    }
}

#[allow(clippy::enum_variant_names)]
enum ProcMsg {
    FromChild {
        child_id: u64,
        err: Box<dyn Any + Send>,
        ack: Sender<()>,
    },

    FromParent(ProcAction),
    FromHandle(ProcAction),
}

enum ProcAction {
    RestartIn(Duration),
    Stop,
}

fn calc_backoff(max: Option<u32>, backoff: Option<Backoff>, attempts: u32) -> Option<Duration> {
    let should_restart = match max {
        None => true,
        Some(m) => attempts < m,
    };

    if should_restart {
        let dur = match backoff {
            None => Duration::ZERO,
            Some(Backoff::Static(dur)) => dur,
            Some(Backoff::Incremental { steps, max }) => cmp::min(steps * attempts, max),
        };

        Some(dur)
    } else {
        None
    }
}

fn spawn<Parent, Child>(mut ctx: Ctx<Child>, delay: Option<Duration>)
where
    Parent: Process,
    Child: Process,
{
    tokio::spawn(async move {
        if let Some(d) = delay {
            time::sleep(d).await;
        }

        // TODO: keep restart count per child
        match Child::init(&mut ctx).await {
            Err(e) => {
                let (tx, rx) = flume::unbounded();
                let _ = ctx.parent_proc_msg_tx.send(ProcMsg::FromChild {
                    child_id: ctx.id,
                    err: Box::new(e),
                    ack: tx,
                });
                let _ = rx.recv_async().await;
            }

            Ok(mut process) => {
                let mut delay = None;

                loop {
                    tokio::select! {
                        biased;

                        proc_msg = ctx.prox_msg_rx.recv_async() => {
                            match proc_msg {
                                Err(_) => break,

                                Ok(ProcMsg::FromHandle(ProcAction::Stop) | ProcMsg::FromParent(ProcAction::Stop)) => {
                                    for child in ctx.children_proc_msg_tx.values() {
                                        let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                                    }

                                    time::sleep(Duration::ZERO).await;

                                    break
                                },

                                Ok(ProcMsg::FromParent(ProcAction::RestartIn(dur))) => {
                                    delay = Some(dur);
                                    break;
                                }

                                Ok(ProcMsg::FromChild { child_id, err, ack }) => {
                                    ctx.handle_err(err, child_id).await;
                                    let _ = ack.send(());
                                }

                                Ok(_) => ()
                            }
                        }

                        recvd = ctx.msg_rx.recv_async() => {
                            match recvd {
                                Err(_) => break,

                                Ok(msg) => {
                                    if let Err(e) = process.handle(msg, &mut ctx).await {
                                        let (tx, rx) = flume::unbounded();
                                        let _ = ctx.parent_proc_msg_tx.send(ProcMsg::FromChild {
                                            child_id: ctx.id,
                                            err: Box::new(e),
                                            ack: tx,
                                        });
                                        let _ = rx.recv_async().await;
                                    };
                                }
                            }
                        }
                    }
                }

                process.exit(&mut ctx).await;

                if delay.is_some() {
                    spawn::<Parent, Child>(ctx, delay)
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

        let mut ctx: Ctx<P> = Ctx {
            id: 0,
            total_children: 0,
            props,
            handle: handle.clone(),
            msg_rx,
            parent_proc_msg_tx: ignore,
            prox_msg_rx: directive_rx,
            children_proc_msg_tx: Default::default(),
            supervision: P::supervision(),
        };

        tokio::spawn(async move {
            match P::init(&mut ctx).await {
                Err(_) => {}

                Ok(mut process) => {
                    loop {
                        tokio::select! {
                            biased;

                            proc_msg = ctx.prox_msg_rx.recv_async() => {
                                match proc_msg {
                                    Err(_) => break,

                                    Ok(ProcMsg::FromHandle(ProcAction::Stop) | ProcMsg::FromParent(ProcAction::Stop)) => {
                                        break
                                    },

                                    Ok(ProcMsg::FromChild { child_id, err, ack }) => {
                                        ctx.handle_err(err, child_id).await;
                                        let _ = ack.send(());
                                    }

                                    _ => {}
                                }
                            }

                            recvd = ctx.msg_rx.recv_async() => {
                                match recvd {
                                    Err(_) => break,

                                    Ok(msg) => {
                                        if  process.handle(msg, &mut ctx).await.is_err() {
                                            break;
                                        };
                                    }
                                }
                            }
                        }
                    }

                    process.exit(&mut ctx).await;
                    for child in ctx.children_proc_msg_tx.values() {
                        let _ = child.send(ProcMsg::FromParent(ProcAction::Stop));
                    }
                }
            }
        });

        self.children_directive_tx.push(handle.proc_msg_tx.clone());

        handle
    }
}
