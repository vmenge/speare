use speare::*;

struct Ping;
struct Pong;

struct ProcA {
    b_pid: Pid<ProcB>,
}

#[process]
impl ProcA {
    #[handler]
    async fn ping(&mut self, _msg: Ping, ctx: &Ctx<Self>) -> Reply<(), ()> {
        println!("ping!");
        ctx.tell(&self.b_pid, Pong).await;

        reply(())
    }
}

struct ProcB;

#[process]
impl ProcB {
    #[handler]
    async fn pong(&mut self, _msg: Pong) -> Reply<(), ()> {
        println!("pong!");

        reply(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    let b_pid = node.spawn(ProcB).await;
    let a_pid = node.spawn(ProcA { b_pid }).await;

    node.tell(&a_pid, Ping).await;

    // ping!
    // pong!
}
