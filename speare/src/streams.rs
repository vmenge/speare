use futures_core::Stream;
use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

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

impl<T> Stream for NoStream<T> {
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<T>> {
        Poll::Pending
    }
}
