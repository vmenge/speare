use std::time::Duration;

use speare::*;

#[derive(Clone)]
struct NoCtxTest;
struct WithCtxTest;

struct MyProc<T>
where
    T: Sync + Send,
{
    t: T,
}

#[process]
impl<T> MyProc<T>
where
    T: Send + Sync,
{
    #[subscriptions]
    async fn sub(&self, evt: &EventBus<Self>) {
        evt.subscribe::<NoCtxTest>().await;
    }

    #[handler]
    async fn no_ctx_test(&mut self, _: NoCtxTest) -> Reply<(), ()> {
        println!("hi");

        noreply()
    }

    #[handler]
    async fn with_ctx_test(&mut self, _: WithCtxTest, ctx: &Ctx<Self>) -> Reply<(), ()> {
        noreply()
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    node.spawn(MyProc { t: 0 }).await;

    node.publish(NoCtxTest).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
}
