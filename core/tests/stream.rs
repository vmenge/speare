use async_trait::async_trait;
use derive_more::From;
use futures::stream;
use speare::{Ctx, Node, Process, Request, StreamHandle};
use std::{marker::PhantomData, time::Duration};
use tokio::time;

struct Streamer {
    handle: StreamHandle,
    state: Vec<i32>,
}

#[derive(From)]
enum Msg {
    #[from]
    Push(i32),
    #[from]
    GetStreamHandle(Request<(), StreamHandle>),
    GetState(Request<(), Vec<i32>>),
}

#[async_trait]
impl Process for Streamer {
    type Props = ();
    type Msg = Msg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let sink = ctx.this().clone();
        let handle = ctx
            .stream(|| async { stream::iter(vec![1, 2, 3].into_iter().map(Ok::<i32, ()>)) })
            .sink(sink)
            .spawn();

        Ok(Streamer {
            handle,
            state: vec![],
        })
    }

    async fn handle(&mut self, msg: Self::Msg, _: &mut Ctx<Self>) -> Result<(), Self::Err> {
        use Msg::*;
        match msg {
            Push(x) => self.state.push(x),
            GetStreamHandle(req) => req.reply(self.handle.clone()),
            GetState(req) => req.reply(self.state.clone()),
        }

        Ok(())
    }
}

#[tokio::test]
async fn it_terminates_process_after_streaming() {
    use Msg::*;
    let node = Node::default();
    let streamer = node.spawn::<Streamer>(());

    time::sleep(Duration::from_millis(100)).await;
    let vec = streamer.reqw(GetState, ()).await.unwrap();
    let handle = streamer.reqw(GetStreamHandle, ()).await.unwrap();

    assert_eq!(vec, vec![1, 2, 3]);
    assert!(!handle.is_alive());
}

