use crate::Actor;
use futures_core::Stream;
use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::time::Interval;

pub trait Sources<A: Actor>: Stream<Item = A::Msg> + Send + Unpin + 'static {}
impl<S, A: Actor> Sources<A> for S where S: Stream<Item = A::Msg> + Send + Unpin + 'static {}

pub struct SourceSet<S>(S);

impl<M: Send + 'static> SourceSet<NoStream<M>> {
    pub fn new() -> Self {
        SourceSet(NoStream(PhantomData))
    }
}

impl<M: Send + 'static> Default for SourceSet<NoStream<M>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> SourceSet<S> {
    pub fn interval<F, M>(self, interval: Interval, f: F) -> SourceSet<Merge<S, IntervalStream<F>>>
    where
        F: Fn() -> M + Send + Unpin + 'static,
    {
        SourceSet(Merge {
            a: self.0,
            b: IntervalStream { interval, f },
        })
    }

    pub fn stream<S2>(self, stream: S2) -> SourceSet<Merge<S, S2>> {
        SourceSet(Merge {
            a: self.0,
            b: stream,
        })
    }
}

impl<S> Stream for SourceSet<S>
where
    S: Stream + Unpin,
{
    type Item = S::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut self.get_mut().0).poll_next(cx)
    }
}

pub struct Merge<A, B> {
    pub a: A,
    pub b: B,
}

impl<T, A, B> Stream for Merge<A, B>
where
    A: Stream<Item = T> + Unpin,
    B: Stream<Item = T> + Unpin,
{
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let this = self.get_mut();

        match Pin::new(&mut this.a).poll_next(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(item)),
            Poll::Ready(None) => Pin::new(&mut this.b).poll_next(cx),
            Poll::Pending => match Pin::new(&mut this.b).poll_next(cx) {
                Poll::Ready(Some(item)) => Poll::Ready(Some(item)),
                _ => Poll::Pending,
            },
        }
    }
}

pub struct NoStream<T>(pub PhantomData<T>);

impl<T> Unpin for NoStream<T> {}

impl<T> Stream for NoStream<T> {
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<T>> {
        Poll::Pending
    }
}

pub struct IntervalStream<F> {
    pub interval: Interval,
    pub f: F,
}

impl<F, M> Stream for IntervalStream<F>
where
    F: Fn() -> M + Unpin,
{
    type Item = M;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<M>> {
        self.interval.poll_tick(cx).map(|_| Some((self.f)()))
    }
}
