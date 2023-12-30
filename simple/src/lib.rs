use async_trait::async_trait;
use flume::{Receiver, Sender};
use std::{any::Any, cmp, time::Duration};
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
            strategy: Strategy::OneForOne,
            directive: Directive::Restart,
            max_restarts: None,
            backoff: None,
            deciders: vec![],
        }
    }

    pub fn one_for_all() -> Self {
        Self {
            strategy: Strategy::OneForOne,
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

#[derive(Clone, Copy, Debug)]
pub enum Strategy {
    OneForOne,
    OneForAll,
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
    directive_tx: Sender<(Strategy, Directive)>,
}

impl<Msg> Clone for Handle<Msg> {
    fn clone(&self) -> Self {
        Self {
            msg_tx: self.msg_tx.clone(),
            directive_tx: self.directive_tx.clone(),
        }
    }
}

impl<Msg> Handle<Msg> {
    pub fn stop(&self) {
        let _ = self
            .directive_tx
            .send((Strategy::OneForAll, Directive::Stop));
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
    props: P::Props,
    handle: Handle<P::Msg>,
    msg_rx: Receiver<P::Msg>,
    parent_directive_tx: Sender<(Strategy, Directive)>,
    directive_rx: Receiver<(Strategy, Directive)>,
    children_directive_tx: Vec<Sender<(Strategy, Directive)>>,
    restarts: u32,
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
        let (msg_tx, msg_rx) = flume::unbounded(); // child
        let (directive_tx, directive_rx) = flume::unbounded(); // child

        let handle = Handle {
            msg_tx,
            directive_tx,
        };

        let ctx: Ctx<Child> = Ctx {
            props,
            handle: handle.clone(),
            msg_rx,
            parent_directive_tx: self.handle.directive_tx.clone(),
            directive_rx,
            children_directive_tx: Default::default(),
            restarts: 0,
        };

        spawn::<P, Child>(ctx, None);

        self.children_directive_tx.push(handle.directive_tx.clone());

        handle
    }

    async fn handle_err<Parent: Process>(&self, e: P::Err) -> ProcAction {
        let sup = Parent::supervision();
        let e: Box<dyn Any + Send> = Box::new(e);
        let directive = sup
            .deciders
            .iter()
            .find_map(|f| f(&e))
            .unwrap_or(sup.directive);

        match (sup.strategy, directive) {
            (Strategy::OneForOne, Directive::Restart) => {
                calc_backoff(sup.max_restarts, sup.backoff, self.restarts)
                    .map(ProcAction::RestartIn)
                    .unwrap_or_else(|| match directive {
                        Directive::Stop => ProcAction::Stop,
                        _ => ProcAction::Nothing,
                    })
            }

            (Strategy::OneForOne, Directive::Stop) => ProcAction::Stop,

            (Strategy::OneForAll, directive) => {
                let _ = self.parent_directive_tx.send((sup.strategy, directive));
                time::sleep(Duration::ZERO).await;
                ProcAction::Nothing
            }

            _ => ProcAction::Nothing,
        }
    }
}

enum ProcAction {
    RestartIn(Duration),
    Nothing,
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

        match Child::init(&mut ctx).await {
            Err(e) => match ctx.handle_err::<Parent>(e).await {
                ProcAction::Nothing => (),
                ProcAction::Stop => (),
                ProcAction::RestartIn(delay) => spawn::<Parent, Child>(ctx, Some(delay)),
            },

            Ok(mut process) => {
                let mut delay = None;

                loop {
                    tokio::select! {
                        biased;

                        directive = ctx.directive_rx.recv_async() => {
                            match directive {
                                Err(_) => break,

                                Ok((_, Directive::Stop)) => {
                                    for child in &ctx.children_directive_tx {
                                        let _ = child.send((Strategy::OneForOne, Directive::Stop));
                                    }

                                    time::sleep(Duration::ZERO).await;

                                    break
                                },

                                // escalated from child
                                Ok((Strategy::OneForAll, Directive::Restart)) => {
                                    for child in &ctx.children_directive_tx {
                                        let _ = child.send((Strategy::OneForOne, Directive::Restart ));
                                    }
                                 },

                                // received from parent
                                Ok((Strategy::OneForOne, Directive::Restart)) => {
                                    delay = calc_backoff(max, backoff, ctx.restarts);
                                    break;
                                 },

                                Ok((_, Directive::Resume)) => ()
                            }
                        }

                        recvd = ctx.msg_rx.recv_async() => {
                            match recvd {
                                Err(_) => break,

                                Ok(msg) => {
                                    if let Err(e) = process.handle(msg, &mut ctx).await {
                                        match ctx.handle_err::<Parent>(e).await {
                                            ProcAction::Nothing => (),
                                            ProcAction::Stop => break,
                                            ProcAction::RestartIn(del) => {
                                                delay = Some(del);
                                                break;
                                            }
                                        };
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
    children_directive_tx: Vec<Sender<(Strategy, Directive)>>,
}

impl Drop for Node {
    fn drop(&mut self) {
        for directive_tx in &self.children_directive_tx {
            let _ = directive_tx.send((Strategy::OneForAll, Directive::Stop));
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
            directive_tx,
        };

        let mut ctx: Ctx<P> = Ctx {
            props,
            handle: handle.clone(),
            msg_rx,
            parent_directive_tx: ignore,
            directive_rx,
            children_directive_tx: Default::default(),
            restarts: 0,
        };

        tokio::spawn(async move {
            match P::init(&mut ctx).await {
                Err(_) => {}

                Ok(mut process) => {
                    loop {
                        tokio::select! {
                            biased;

                            directive = ctx.directive_rx.recv_async() => {
                                match directive {
                                    Err(_) => break,

                                    Ok((_, Directive::Stop)) => break,

                                    // escalated from child
                                    Ok((Strategy::OneForAll, Directive::Restart { max, backoff })) => {
                                        for child in &ctx.children_directive_tx {
                                            let _ = child.send((Strategy::OneForOne, Directive::Restart { max, backoff }));
                                        }
                                    },

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
                    for child in ctx.children_directive_tx {
                        let _ = child.send((Strategy::OneForOne, Directive::Stop));
                    }
                }
            }
        });

        self.children_directive_tx.push(handle.directive_tx.clone());

        handle
    }
}
