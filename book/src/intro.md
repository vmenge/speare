# Introduction

`speare` is a thin abstraction over [tokio tasks](https://tokio.rs/tokio/tutorial/spawning#tasks) and [flume channels](https://github.com/zesterer/flume) for actor-based concurrency in Rust. Each actor lives on its own tokio task, owns its data, and communicates exclusively through messages.

## Why Actors?

If you have written async Rust before, you have likely run into `Arc<Mutex<T>>` for sharing state across tasks. Actors offer an alternative:

- **No shared mutable state.** Each actor owns its data. No `Arc<Mutex<T>>`, no lock contention, no deadlocks from forgetting lock ordering.
- **Natural concurrency boundaries.** One actor = one task = one mailbox. Concurrency is modeled as independent units exchanging messages rather than threads contending over locks.
- **Fault isolation.** When an actor fails, its parent decides what happens: stop, restart, or ignore the error. A bug in one part of the system does not have to bring everything down.
- **Modular design.** Actors enforce separation of concerns. Each actor has a single responsibility, a defined message protocol, and a clear lifecycle.

## A Minimal Example

Below is a complete `Counter` actor. It accepts three kinds of messages: add, subtract, and print.

```rust,ignore
use speare::*;
use std::time::Duration;
use tokio::time;

struct Counter {
    count: u32,
}

enum CounterMsg {
    Add(u32),
    Subtract(u32),
    Print,
}

impl Actor for Counter {
    type Props = ();
    type Msg = CounterMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Counter { count: 0 })
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            CounterMsg::Add(n) => self.count += n,
            CounterMsg::Subtract(n) => self.count -= n,
            CounterMsg::Print => println!("Count is {}", self.count),
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    let counter = node.actor::<Counter>(()).spawn();

    counter.send(CounterMsg::Add(5));
    counter.send(CounterMsg::Subtract(2));
    counter.send(CounterMsg::Print); // will print 3

    // Give the actor time to process messages before the program exits.
    time::sleep(Duration::from_millis(10)).await;
}
```

## What Just Happened?

Two types drive the example above: `Node` and `Handle`.

**`Node`** is the root of an actor hierarchy, created by calling `Node::default()`. You spawn actors from it with `node.actor::<MyActor>(props).spawn()`, where `props` is whatever data the actor needs at initialization time (here just `()`). The `Node` owns all top-level actors and shuts them down when it is dropped.

`.spawn()` returns a **`Handle<Msg>`**. It is a lightweight, cloneable reference to a running actor. You can use it to send messages (`handle.send(msg)`), stop the actor (`handle.stop()`), or check if it is still alive (`handle.is_alive()`). Handles can be passed freely between actors and tasks.

The `Actor` trait itself requires two things: an `init` function that constructs the actor, and a `handle` function that processes each incoming message. Everything else -- lifecycle hooks, supervision, streams -- is optional.

## What This Book Covers

- [The Actor](the_actor.md) -- the `Actor` trait, `Props`, `Msg`, `Err`, and `Ctx` in detail.
- [Communication](communication.md) -- fire-and-forget messages, request-response, and the `From`-based send ergonomics.
- [Lifecycle & Supervision](lifecycle_and_supervision.md) -- init, exit, restart strategies, backoff, and the `watch` callback.
- [Concurrency Patterns](concurrency_patterns.md) -- background tasks, streams, intervals, and `SourceSet`.
- [Service Discovery](service_discovery.md) -- the actor registry, `spawn_registered`, `spawn_named`, and cross-actor lookup.
