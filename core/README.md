# speare
`speare` is a minimalistic actor framework that also has pub / sub capabities.

## Your first `Process`
`speare` revolves around the idea of `Processes`, which have their states isolated to their own `tokio::task`.
Processes need to implement the `Process` trait. To define message handlers you can use the `#[process]` and `#[handler]` attributes.

```rs
use speare::*;

struct IncreaseBy(u64);

#[derive(Default)]
struct Counter {
    num: u64,
}

impl Process for Counter {}

#[process]
impl Counter {
    #[handler]
    async fn increase(&mut self, msg: IncreaseBy, ctx: &Ctx<Self>) -> Result<u64, ()> {
        self.num += msg.0;
        Ok(self.num)
    }
}
```

Arguments for functions with the `#[handler]` attribute should always be: `&mut self`, `msg: M`, `ctx: &Ctx<Self>`.

After defining your `Process`, you can now spawn it in a `Node` and send a fire and forget message with `.tell()`, or wait for a response with `.ask()`.

```rs
#[tokio::main]
async fn main() {
    let node = Node::default();
    let counter_pid = node.spawn(Counter::default()).await;

    node.tell(&counter_pid, IncreaseBy(1)).await;
    node.tell(&counter_pid, IncreaseBy(2)).await;
    let result = node.ask(&counter_pid, IncreaseBy(1)).await.unwrap_or(0);

    assert_eq!(result, 4);
}

```

`Processes` can also have custom behaviour on startup and on termination.

```rs
#[async_trait]
impl Process for Counter {
    async fn on_init(&mut self, ctx: &Ctx<Self>) {
        println!("Hello!");
    }

    async fn on_exit(&mut self, ctx: &Ctx<Self>) {
        println!("Goodbye!");
    }
}
```

If you need to send messages or spawn other processes from inside a `Process`, you can do so using the `Ctx<Self>` reference, which also has all functions availalbe on a `Node` instance.

```rs
#[process]
impl Counter {
    #[handler]
    async fn spawn_another(&mut self, msg: SpawnAnother, ctx: &Ctx<Self>) -> Result<(), ()> {
        ctx.spawn(MyOtherProc::default()).await;
        Ok(())
    }
}
```

To terminate a process you can use the `.exit()` function.

```rs
let node = Node::default();
let counter_pid = node.spawn(Counter::default()).await;
node.exit(&counter_pid).await;
```

## Pub / Sub
Every `Process` that implements a `Handler` for a message `M`, can also manually subscribe to global publishes of that message, the only requirement being that the message must implement `Clone`.

Here is a small example:
```rs
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

```