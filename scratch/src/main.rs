use std::time::Duration;

use speare::*;
use tokio::{task, time};
#[derive(Clone)]
struct SayHi;
struct GiveBone;

#[derive(Default)]
struct Dog {
    hi_responder: Option<Responder<Self, SayHi>>,
}

#[async_trait]
impl Process for Dog {
    type Error = ();

    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }
}

#[process]
impl Dog {
    #[handler]
    async fn hi(&mut self, _msg: SayHi, ctx: &Ctx<Self>) -> Reply<String, ()> {
        self.hi_responder = ctx.responder::<SayHi>();

        noreply()
    }

    #[handler]
    async fn bruh(&mut self, _msg: GiveBone, _ctx: &Ctx<Self>) -> Reply<(), ()> {
        if let Some(x) = &self.hi_responder {
            x.reply(Ok("Hello".to_string()))
        }

        reply(())
    }
}
struct Cat;

#[async_trait]
impl Process for Cat {
    type Error = ();

    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }
}

#[process]
impl Cat {
    #[handler]
    async fn hi(&mut self, _msg: SayHi, _ctx: &Ctx<Self>) -> Reply<(), ()> {
        println!("MEOW!");
        reply(())
    }
}

struct Container<T>(T);

#[async_trait]
impl<T> Process for Container<T>
where
    T: Sync + Send,
{
    type Error = ();

    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }
}

#[process]
impl<T> Container<T>
where
    T: Sync + Send,
{
    #[handler]
    async fn hi(&mut self, _msg: SayHi) -> Reply<(), ()> {
        println!("Hi im container!");
        reply(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    let dog_pid = node.spawn(Dog::default()).await;

    let node_clone = node.clone();
    let dog_pid_clone = dog_pid.clone();
    task::spawn(async move {
        let res = node_clone.ask(&dog_pid_clone, SayHi).await;
        if let Ok(res) = res {
            println!("got response! {}", res);
        }
    });

    time::sleep(Duration::from_millis(10)).await;
    node.tell(&dog_pid, GiveBone).await;
    time::sleep(Duration::from_millis(500)).await;

    // "WOOF!"
    // "MEOW!"
}
