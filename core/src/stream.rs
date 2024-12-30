use crate::{Ctx, Handle, Process};
use async_trait::async_trait;
use futures::{pin_mut, Stream, StreamExt};
use std::future::Future;
use std::marker::PhantomData;

// Streams are spanwed by other processes, which means they are killed when the parent process is killed.

struct Source<F, Fut, S, T, E, Snk> {
    stream: S,
    _marker: std::marker::PhantomData<(F, Fut, S, T, E, Snk)>,
}

pub trait Sink<T> {
    fn consume(&self, item: T) -> impl Future<Output = ()> + Send;
}

struct Props<F, Snk> {
    init: F,
    sink: Snk,
}

// i love rust haha
#[async_trait]
impl<F, Fut, S, T, E, Snk> Process for Source<F, Fut, S, T, E, Snk>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = S> + Send + 'static,
    S: Stream<Item = Result<T, E>> + Send + 'static + Unpin,
    T: Send + 'static,
    E: Send + Sync + 'static,
    Snk: Sink<T> + Send + 'static,
{
    type Props = Props<F, Snk>;
    type Msg = ();
    type Err = E;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let stream = (ctx.props().init)().await;

        Ok(Self {
            stream,
            _marker: PhantomData,
        })
    }

    async fn handle(&mut self, _: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        let stream = &mut self.stream;
        pin_mut!(stream);

        let mut fut = stream.next();

        loop {
            tokio::select! {
                biased;

                msg = ctx.proc_msg_rx.recv_async() => {
                    if let Ok(msg) = msg {
                        let _ = ctx.this().proc_msg_tx.send(msg);
                        break;
                    }
                }

                item = &mut fut => {
                    match item {
                        None => {
                            ctx.this().stop();
                            break;
                        }

                        Some(res) => {
                            ctx.props().sink.consume(res?).await;
                            fut = stream.next();
                        }
                    };
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct StreamHandle {
    handle: Handle<()>,
}

impl StreamHandle {
    pub fn stop(&self) {
        self.handle.stop();
    }

    pub fn is_alive(&self) -> bool {
        self.handle.is_alive()
    }
}

pub struct NoSink;

impl<T> Sink<T> for NoSink
where
    T: Send,
{
    async fn consume(&self, _item: T) {}
}

pub struct StreamBuilder<'a, P, F, Fut, S, T, E, Snk>
where
    P: Process,
{
    init: F,
    sink: Snk,
    ctx: &'a mut Ctx<P>,
    _marker: PhantomData<(Fut, S, T, E)>,
}

impl<'a, P, F, Fut, S, T, E> StreamBuilder<'a, P, F, Fut, S, T, E, NoSink>
where
    P: Process,
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = S> + Send + 'static,
    S: Stream<Item = Result<T, E>> + Send + 'static + Unpin,
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    pub fn new(init: F, ctx: &'a mut Ctx<P>) -> Self {
        Self {
            init,
            sink: NoSink,
            ctx,
            _marker: PhantomData,
        }
    }

    pub fn sink<Snk>(self, sink: Snk) -> StreamBuilder<'a, P, F, Fut, S, T, E, Snk>
    where
        Snk: Sink<T> + Send + Sync + 'static,
    {
        StreamBuilder {
            init: self.init,
            sink,
            ctx: self.ctx,
            _marker: PhantomData,
        }
    }
}

impl<P, F, Fut, S, T, E, Snk> StreamBuilder<'_, P, F, Fut, S, T, E, Snk>
where
    P: Process,
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = S> + Send + 'static,
    S: Stream<Item = Result<T, E>> + Send + 'static + Unpin,
    T: Send + 'static,
    E: Send + Sync + 'static,
    Snk: Sink<T> + Send + 'static,
{
    pub fn spawn(self) -> StreamHandle {
        let handle = self.ctx.spawn::<Source<F, Fut, S, T, E, Snk>>(Props {
            init: self.init,
            sink: self.sink,
        });

        handle.send(());

        StreamHandle { handle }
    }
}

impl<T, K> Sink<K> for Handle<T>
where
    T: Send,
    K: Send + Into<T>,
{
    async fn consume(&self, item: K) {
        let k: T = item.into();
        self.send(k);
    }
}

impl<T, K> Sink<K> for flume::Sender<T>
where
    T: Send,
    K: Send + Into<T>,
{
    async fn consume(&self, item: K) {
        let k: T = item.into();
        let _ = self.send_async(k).await;
    }
}

impl<T, K> Sink<K> for tokio::sync::mpsc::Sender<T>
where
    T: Send,
    K: Send + Into<T>,
{
    async fn consume(&self, item: K) {
        let k: T = item.into();
        let _ = self.send(k).await;
    }
}

impl<T, K> Sink<K> for tokio::sync::broadcast::Sender<T>
where
    T: Send,
    K: Send + Into<T>,
{
    async fn consume(&self, item: K) {
        let k: T = item.into();
        let _ = self.send(k);
    }
}
