use anyhow::Context;
use speare::*;
use std::time::Duration;

struct SendRequest;

struct HttpRequester;

#[process]
impl HttpRequester {
    #[on_init]
    async fn init(&mut self, ctx: &Ctx<Self>) {
        ctx.tell(ctx.this(), SendRequest).await;
    }

    #[handler]
    async fn send_req(&mut self, _msg: SendRequest, ctx: &Ctx<Self>) -> Reply<(), ()> {
        // http call here

        ctx.tell_in(ctx.this(), SendRequest, Duration::from_secs(3_600))
            .await;

        reply(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    let x = node.spawn(HttpRequester).await;
    let z = node.ask(&x, SendRequest).await.context("yoo");
}
