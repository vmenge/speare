# speare
`speare` is a minimalistic actor framework that also has pub / sub capabities.

## Why `speare`?
`speare` is simply an abstraction over tokio threads and channels, offering functionality to manage these threads, and pass messages between these in a more practical manner. The question instead should be: *"why message passing (channels) instead of sharing state (e.g. `Arc<Mutex<T>>`)?"*

- **Easier reasoning**: With message passing, each piece of data is owned by a single thread at a time, making the flow of data and control easier to reason about.
- **Deadlock Prevention**: Shared state with locks (like mutexes) can lead to deadlocks if not managed carefully. Message passing, especially in Rust, is less prone to deadlocks as it doesn’t involve traditional locking mechanisms.
- **Encouragement of Decoupled Design**: Message passing promotes a more modular, decoupled design where components communicate through well-defined interfaces (channels), enhancing maintainability and scalability of the code.


## Your first `Process`
`speare` revolves around the idea of `Processes`, which have their states isolated to their own `tokio::task`.
To create a `Process`, use the `#[process]` macro on the impl block of a type. 

```rust
use speare::*;

struct Counter {
    num: u64,
}

#[process]
impl Counter {}
```

Now you can spawn your process!

```rust
#[tokio::main]
async fn main() {
    let node = Node::default();
    let counter_pid = node.spawn(Counter { num: 0}).await;
}
```

But this is sort of a useless `Process`, isn't it?
Let's give it a `Handler` by using the `#[handler]` macro.

```rust
use speare::*;

struct IncreaseBy(u64);

struct Counter {
    num: u64,
}

#[process]
impl Counter {
    #[handler]
    async fn increase(&mut self, msg: IncreaseBy) -> Reply<u64, ()> {
        self.num += msg.0;

        reply(self.num)
    }
}
```

Arguments for functions with the `#[handler]` attribute should always be: `&mut self`, `msg: M`, and optionally `ctx: &Ctx<Self>`. A `Handler` should always have a return type of `Reply<T, E>`.

After defining your `Process`, you can now spawn it in a `Node` and send a fire and forget message with `.tell()`, or wait for a response with `.ask()`.

```rust
#[tokio::main]
async fn main() {
    let node = Node::default();
    let counter_pid = node.spawn(Counter { num: 0 }).await;

    node.tell(&counter_pid, IncreaseBy(1)).await;
    node.tell(&counter_pid, IncreaseBy(2)).await;
    let result = node.ask(&counter_pid, IncreaseBy(1)).await.unwrap_or(0);

    assert_eq!(result, 4);
}

```

Processes can also have custom behaviour on startup and on termination by using the `#[on_init]` and `#[on_exit]` macros. Note the signature of the functions, they should be the same as in the example below.

```rust
use speare::*;

struct IncreaseBy(u64);

struct Counter {
    num: u64,
}

#[process]
impl Counter {
    #[on_init]
    async fn init(&mut self, ctx: &Ctx<Self>) {
        println!("Hello!");
    }

    #[on_exit]
    async fn exit(&mut self, ctx: &Ctx<Self>) {
        println!("Goodbye!");
    }

    #[handler]
    async fn increase(&mut self, msg: IncreaseBy) -> Reply<u64, ()> {
        self.num += msg.0;

        reply(self.num)
    }
}
```

If you need to send messages or spawn other processes from inside a `Process`, you can do so using a `Ctx<Self>` reference, which also has all functions availalbe on a `Node` instance. Every `Handler` created with the `#[handler]` macro can have a `&Ctx<Self>` as an optional third argument. If you're not using it, you can omit it and the macro will take care of it for you.

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
Every `#[handler]` must return a `Reply<T,E>`, which is just an alias for `Result<Option<T>, E>`. You can still use `?` to early return errors from your `#[handler]` functions. There are two helper functions in `speare` for creating Replies:

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
To avoid replying immediately or even in the same `#[handler]` that receives a message, you can use `noreply()` on the original `#[handler]`, and store the `Responder` for that message to send the response later. If the `Responder` is not stored and `noreply()` is used, calling `.ask()` on that `#[handler]` will always fail. In the example below you can see `Dog` will only reply to `SayHi` once it is given a bone with the `GiveBone` message.

```rust
use speare::*;

struct SayHi;
struct GiveBone;

#[derive(Default)]
struct Dog {
    hi_responder: Option<Responder<Self, SayHi>>,
}

#[process]
impl Dog {
    // the hi Handler specifies that it returns a String as a response,
    // thus when we call it on get_bone, the Reply sent through the 
    // Responder must be a String.
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

#[process]
impl Dog {
    #[subscriptions]
    async fn subs(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }

    #[handler]
    async fn hi(&mut self, msg: SayHi, ctx: &Ctx<Self>) -> Reply<(), ()> {
        println!("WOOF!");
        reply(())
    }
}

struct Cat;

#[process]
impl Cat {
    #[subscriptions]
    async fn subs(&self, evt: &EventBus<Self>) {
        evt.subscribe::<SayHi>().await;
    }

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