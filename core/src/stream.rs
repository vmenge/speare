use crate::{Ctx, Process};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::future::Future;
use std::marker::PhantomData;

struct Source<F, Fut, S, T, E, Snk> {
    stream: S,
    _marker: std::marker::PhantomData<(F, Fut, S, T, E, Snk)>,
}

trait Sink<T> {
    fn send(&self, item: T);
}

struct Props<F, Snk> {
    init: F,
    sink: Snk,
}

// i love rust haha
#[async_trait]
impl<F, Fut, S, T, E, Snk> Process for Source<F, Fut, S, T, E, Snk>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = S> + Send + Sync + 'static,
    S: Stream<Item = Result<T, E>> + Send + Sync + 'static + Unpin,
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
    Snk: Sink<T> + Send + Sync + 'static,
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
        let mut fut = self.stream.next();

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
                            ctx.props().sink.send(res?);
                            fut = self.stream.next();
                        }
                    };
                }
            }
        }

        Ok(())
    }
}
