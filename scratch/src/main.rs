use speare::*;

#[derive(Clone)]
struct SayHi;

struct Dog;

#[async_trait]
impl Process for Dog {
    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }
}

#[process]
impl Dog {
    #[handler]
    async fn hi(&mut self, msg: SayHi, ctx: &Ctx<Self>) -> Result<(), ()> {
        println!("WOOF!");
        Ok(())
    }
}
struct Cat;

#[async_trait]
impl Process for Cat {
    async fn subscriptions(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }
}

#[process]
impl Cat {
    #[handler]
    async fn hi(&mut self, msg: SayHi, ctx: &Ctx<Self>) -> Result<(), ()> {
        println!("MEOW!");
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    node.spawn(Cat).await;
    node.spawn(Dog).await;

    node.publish(SayHi).await;

    // "WOOF!"
    // "MEOW!"
}
