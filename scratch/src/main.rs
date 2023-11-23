use std::time::Duration;

use speare::*;

struct Quitter;

#[process(Error = String)]
impl Quitter {
    #[handler]
    async fn do_it(&mut self, msg: String, ctx: &Ctx<Self>) -> Reply<(), ()> {
        println!("doing {}", msg);

        ctx.tell_in(ctx.this(), "bla".to_string(), Duration::from_millis(200))
            .await;

        reply(())
    }
}

struct Supervisor(Pid<Quitter>);

#[process]
impl Supervisor {
    #[on_init]
    async fn init(&mut self, ctx: &Ctx<Self>) {
        println!("MONITORING!!");
        ctx.monitor(&self.0);
    }

    #[handler]
    async fn handle_exit(&mut self, msg: ExitSignal<Quitter>) -> Reply<(), ()> {
        println!("QUITTING! {:?}", msg);
        reply(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    let quitter = node.spawn(Quitter).await;
    node.spawn(Supervisor(quitter.clone())).await;
    node.tell(&quitter, "ble".to_string()).await;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    node.exit(&quitter, ExitReason::Shutdown).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}
