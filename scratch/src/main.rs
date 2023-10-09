use speare::*;

struct IncreaseBy(u64);

#[derive(Default)]
struct Counter {
    num: u64,
}

#[async_trait]
impl Process for Counter {
    async fn on_init(&mut self, ctx: &Ctx<Self>) {
        println!("Hello!");
    }

    async fn on_exit(&mut self, ctx: &Ctx<Self>) {
        println!("Goodbye!");
    }
}

#[process]
impl Counter {
    #[handler]
    async fn spawn_another(&mut self, msg: IncreaseBy, ctx: &Ctx<Self>) -> Result<(), ()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    let counter_pid = node.spawn(Counter::default()).await;
    node.tell(&counter_pid, IncreaseBy(1)).await;
    node.tell(&counter_pid, IncreaseBy(2)).await;
    let result = node.ask(&counter_pid, IncreaseBy(1)).await.unwrap_or(0);
}
