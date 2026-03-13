# Communication

This chapter covers how to create an actor system, spawn actors, send messages, and get replies.

## Node

Every actor system starts with a `Node`. It is the root supervisor -- all actors you spawn from it become its children.

```rust,ignore
let mut node = Node::default(); // or Node::new()
```

Spawn an actor by calling `node.actor::<MyActor>(props).spawn()`, which returns a `Handle<Msg>`:

```rust,ignore
let counter = node.actor::<Counter>(()).spawn();
```

When a `Node` is dropped, it sends a stop signal to all its children in a fire-and-forget fashion -- it does not wait for them to finish. If you need to wait for actors to finish processing their remaining messages, call `shutdown()`:

```rust,ignore
node.shutdown().await;
```

## Handle

`Handle<Msg>` is what you get back from spawning an actor. It is your only way to interact with that actor from the outside. `Handle` is `Clone`, so you can share it freely.

| Method | Description |
|---|---|
| `handle.send(msg)` | Fire-and-forget. Silently fails if the actor is dead. |
| `handle.send_in(msg, duration)` | Sends `msg` after the given `Duration`. |
| `handle.stop()` | Requests the actor to stop (non-blocking). |
| `handle.restart()` | Requests the actor to restart (non-blocking). |
| `handle.is_alive()` | Returns `true` if the actor is still running. |

Here is a complete example spawning a `Counter` and using its handle:

```rust,ignore
use speare::*;
use std::time::Duration;
use tokio::time;

// (assume Counter actor defined as in previous chapter)

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    let counter = node.actor::<Counter>(()).spawn();

    counter.send(CounterMsg::Add(10));
    counter.send(CounterMsg::Print);

    assert!(counter.is_alive());

    counter.stop();
    task::yield_now().await;
    assert!(!counter.is_alive());

    node.shutdown().await;
}
```

`send` is non-blocking. Messages are queued and processed one at a time by the actor. If the actor has already stopped, `send` does nothing -- it will not panic or return an error.

## The `From` Trick

`Handle::send` accepts any type `M` where `Msg: From<M>`. If you derive `From` on your message enum (via [derive_more](https://docs.rs/derive_more)), you can send inner types directly without wrapping them:

```rust,ignore
use derive_more::From;

#[derive(From)]
enum Msg {
    Increment(u32),
    SetName(String),
}

// Both work:
handle.send(Msg::Increment(1));
handle.send(1u32); // auto-converted via From<u32>
handle.send("Alice".to_string()); // auto-converted via From<String>
```

This keeps call sites clean, especially when variants are simple newtypes. It also plays a central role in request-response, as shown next.

## Request-Response

Sometimes fire-and-forget is not enough -- you need an answer back. `speare` provides `Request<Req, Res>` for this.

The pattern has two sides:

**Sender side** -- use `handle.req(payload).await`, which returns `Result<Res, ReqErr>`:

```rust,ignore
let count: u32 = handle.req(()).await?;
```

**Receiver side** -- match on the `Request` variant inside `handle()` and call `req.reply(value)`:

```rust,ignore
KvMsg::Get(req) => {
    let value = self.data.get(req.data()).cloned();
    req.reply(value);
}
```

`req.data()` gives you a reference to the request payload. `req.reply(value)` sends the response back to the caller.

### Defining request variants

Add `Request<Req, Res>` as a variant in your message enum:

```rust,ignore
use speare::*;
use derive_more::From;

#[derive(From)]
enum Msg {
    GetCount(Request<(), u32>),
}
```

Because `Msg` derives `From`, the `req` method on `Handle` can automatically wrap a `Request<(), u32>` into `Msg::GetCount`. If you cannot derive `From` for a particular variant, use `reqw` instead:

```rust,ignore
let count: u32 = handle.reqw(Msg::GetCount, ()).await?;
```

`reqw` takes a wrapper function (here the enum variant constructor `Msg::GetCount`) and the request payload as separate arguments.

### Timeouts

By default, `req` waits indefinitely. Use `req_timeout` to set a deadline:

```rust,ignore
let count: u32 = handle.req_timeout((), Duration::from_secs(1)).await?;
```

A `reqw_timeout` variant is also available:

```rust,ignore
let count: u32 = handle.reqw_timeout(Msg::GetCount, (), Duration::from_secs(1)).await?;
```

### Error handling

`req` and its variants return `Result<Res, ReqErr>`. There are two failure cases:

- `ReqErr::Dropped` -- the actor died (or the `Request` was dropped) before calling `reply`.
- `ReqErr::Timeout` -- the deadline passed before a response arrived.

### Full example: a key-value store

Putting it all together with a KV store actor that supports both fire-and-forget writes and request-response reads:

```rust,ignore
use speare::*;
use std::collections::HashMap;
use derive_more::From;

struct KvStore {
    data: HashMap<String, String>,
}

#[derive(From)]
enum KvMsg {
    Set(SetCmd),
    Get(Request<String, Option<String>>),
}

struct SetCmd {
    key: String,
    value: String,
}

impl Actor for KvStore {
    type Props = ();
    type Msg = KvMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(KvStore { data: HashMap::new() })
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            KvMsg::Set(cmd) => {
                self.data.insert(cmd.key, cmd.value);
            }

            KvMsg::Get(req) => {
                let value = self.data.get(req.data()).cloned();
                req.reply(value);
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    let kv = node.actor::<KvStore>(()).spawn();

    kv.send(SetCmd { key: "name".into(), value: "Alice".into() });

    let name = kv.req("name".to_string()).await.unwrap();
    println!("{name:?}"); // Some("Alice")

    node.shutdown().await;
}
```

Notice how `kv.send(SetCmd { ... })` works without wrapping in `KvMsg::Set` -- the `From` derive handles the conversion. Likewise, `kv.req("name".to_string())` automatically wraps the `Request<String, Option<String>>` into `KvMsg::Get`.
