# speare
`speare` is a minimalistic actor framework that also has pub / sub capabities.

## Your first `Process`
`speare` revolves around the idea of `Processes`, which have their states isolated to their own `tokio::task`.
Processes need to implement the `Process` trait. To define message handlers you can use the `#[process]` and `#[handler]` attributes.

```rust
use speare::*;

struct IncreaseBy(u64);

#[derive(Default)]
struct Counter {
    num: u64,
}

impl Process for Counter {
    type Error = ();
}

#[process]
impl Counter {
    #[handler]
    async fn increase(&mut self, msg: IncreaseBy, ctx: &Ctx<Self>) -> Reply<u64, ()> {
        self.num += msg.0;
        reply(self.num)
    }
}
```

Arguments for functions with the `#[handler]` attribute should always be: `&mut self`, `msg: M`, `ctx: &Ctx<Self>`.

After defining your `Process`, you can now spawn it in a `Node` and send a fire and forget message with `.tell()`, or wait for a response with `.ask()`.

```rust
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

```rust
#[async_trait]
impl Process for Counter {
    type Error = ();

    async fn on_init(&mut self, ctx: &Ctx<Self>) {
        println!("Hello!");
    }

    async fn on_exit(&mut self, ctx: &Ctx<Self>) {
        println!("Goodbye!");
    }
}
```

If you need to send messages or spawn other processes from inside a `Process`, you can do so using the `Ctx<Self>` reference, which also has all functions availalbe on a `Node` instance.

```rust
#[process]
impl Counter {
    #[handler]
    async fn spawn_another(&mut self, msg: SpawnAnother, ctx: &Ctx<Self>) -> Reply<(), ()> {
        ctx.spawn(MyOtherProc::default()).await;
        reply(())
    }
}
```

To terminate a process you can use the `.exit()` function.

```rust
let node = Node::default();
let counter_pid = node.spawn(Counter::default()).await;
node.exit(&counter_pid, ExitReason::Shutdown).await;
```

## The `Reply` type
Every `#[handler]` must return a `Reply<T,E>`, which is just an alias for `Result<Option<T>, E>`. You can still use `?` to early return errors from your `#[handler]` functions. There are two helper functions for creating Replies:

```rust
pub fn reply<T, E>(item: T) -> Result<Option<T>, E> {
    Ok(Some(item))
}

pub fn noreply<T, E>() -> Result<Option<T>, E> {
    Ok(None)
}
```

`noreply()` will make it impossible for `.ask()` to succeed on that specific handler, unless an instance of a `Responder` is stored somewhere.

## Deferring Replies
To avoid replying immediately or even in the same `#[handler]` that receives a message, you can use `noreply()` on the original `#[handler]`, and store the responder for that message to send the response later. If the `Responder` is not stored and `noreply()` is used, calling `.ask()` on that `#[handler]` will always fail. In the example below you can see `Dog` will only reply to `SayHi` once it is given a bone with the `GiveBone` message.

```rust
use speare::*;

struct SayHi;
struct GiveBone;

#[derive(Default)]
struct Dog {
    hi_responder: Option<Responder<Self, SayHi>>,
}

impl Process for Dog {
    type Error = ();
}

#[process]
impl Dog {
    // the hi Handler specifies that it returns a String as a response,
    // thus when we call it on get_bone, the Reply itself must be a String.
    #[handler]
    async fn hi(&mut self, _msg: SayHi, ctx: &Ctx<Self>) -> Reply<String, ()> {
        self.hi_responder = ctx.responder::<SayHi>();

        noreply()
    }

    #[handler]
    async fn get_bone(&mut self, _msg: GiveBone, _ctx: &Ctx<Self>) -> Reply<(), ()> {
        if let Some(responder) = &self.hi_responder {
            responder.reply(Ok("Hello".to_string()))
        }

        noreply() // this could also be a reply(()), but noreply() conveys meaning better
    }
}
```

## Pub / Sub
Every `Process` that implements a `Handler` for a message `M`, can also manually subscribe to global publishes of that message, the only requirement being that the message must implement `Clone`.

Here is a small example:
```rust
use speare::*;

#[derive(Clone)]
struct SayHi;

struct Dog;

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
    async fn hi(&mut self, msg: SayHi, ctx: &Ctx<Self>) -> Reply<(), ()> {
        println!("WOOF!");
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
    async fn hi(&mut self, msg: SayHi, ctx: &Ctx<Self>) -> Reply<(), ()> {
        println!("MEOW!");
        reply(())
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