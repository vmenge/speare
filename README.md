# speare

[![crates.io](https://img.shields.io/crates/v/speare.svg)](https://crates.io/crates/speare)
[![docs.rs](https://docs.rs/speare/badge.svg)](https://docs.rs/speare)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

`speare` is a minimalistic actor framework. A thin abstraction over tokio tasks and flume channels with supervision inspired by Akka.NET. Read [The Speare Book](https://vmenge.github.io/speare/) for more details on how to use the library.

## Quick Look
A minimal `Counter` actor:

```rust
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

    time::sleep(Duration::from_millis(10)).await;
}
```

## Props & Request-Response

Props, request-response via `Request<Req, Res>`, and the `From` trick for ergonomic sends:

```rust
use speare::*;
use derive_more::From;

struct Counter {
    count: u32,
}

struct CounterProps {
    initial_count: u32,
}

#[derive(From)]
enum CounterMsg {
    Add(u32),
    GetCount(Request<(), u32>),
}

impl Actor for Counter {
    type Props = CounterProps;
    type Msg = CounterMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Counter {
            count: ctx.props().initial_count,
        })
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            CounterMsg::Add(n) => self.count += n,
            CounterMsg::GetCount(req) => req.reply(self.count),
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    let counter = node
        .actor::<Counter>(CounterProps { initial_count: 10 })
        .spawn();

    counter.send(CounterMsg::Add(5));
    counter.send(7u32); // auto-converted to CounterMsg::Add via From

    let count: u32 = counter.req(()).await.unwrap();
    println!("Count is {count}"); // 22

    node.shutdown().await;
}
```

## PubSub & Registry

Type-safe publish/subscribe across actors, and a global registry for finding actors by type or name:

```rust
use speare::*;
use derive_more::From;

// PubSub messages must be Clone
#[derive(Clone)]
struct Event(String);

// Logger subscribes to the "events" topic
struct Logger;

#[derive(From)]
enum LoggerMsg {
    Event(Event),
}

impl Actor for Logger {
    type Props = ();
    type Msg = LoggerMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        ctx.subscribe::<Event>("events").unwrap();
        Ok(Logger)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            LoggerMsg::Event(Event(e)) => println!("[log] {e}"),
        }

        Ok(())
    }
}

// Greeter is registered so others can find it without holding a Handle
struct Greeter;

impl Actor for Greeter {
    type Props = ();
    type Msg = String;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Greeter)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        let greeting = format!("Hello, {msg}!");

        // publish to all "events" subscribers
        ctx.publish("events", Event(greeting)).unwrap();

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut node = Node::default();
    node.actor::<Logger>(()).spawn();
    node.actor::<Greeter>(()).spawn_registered().unwrap();

    // look up Greeter by type and send — no Handle needed
    node.send::<Greeter>("world".to_string()).unwrap();
    // Logger will print: [log] Hello, world!

    // you can also use spawn_named("name") and send_to::<Msg>("name", msg)
    node.shutdown().await;
}
```

## Supervision

Spawn supervised children with automatic restarts, backoff, and a watch callback for when retries are exhausted:

```rust
use speare::*;
use std::time::Duration;

// inside a parent actor's init:
let worker = ctx.actor::<Worker>(())
    .supervision(Supervision::Restart {
        max: Limit::Amount(3),
        backoff: Backoff::Incremental {
            min: Duration::from_millis(100),
            max: Duration::from_secs(5),
            step: Duration::from_millis(500),
        },
    })
    .watch(|err| ManagerMsg::WorkerDied(format!("{err:?}")))
    .spawn();
```


## Why `speare`?
`speare` is a minimal abstraction layer over [tokio green threads](https://tokio.rs/tokio/tutorial/spawning#tasks) and [flume channels](https://github.com/zesterer/flume), offering functionality to manage these threads, and pass messages between these in a more practical manner. The question instead should be: *"why message passing (channels) instead of sharing state (e.g. `Arc<Mutex<T>>`)?"*

- **Easier reasoning**: With message passing, each piece of data is owned by a single thread at a time, making the flow of data and control easier to reason about.
- **Deadlock Prevention**: Shared state with locks (like mutexes) can lead to deadlocks if not managed carefully. Message passing, especially in Rust, is less prone to deadlocks as it doesn’t involve traditional locking mechanisms.
- **Encouragement of Decoupled Design**: Message passing promotes a more modular, decoupled design where components communicate through well-defined interfaces (channels), enhancing maintainability and scalability of the code.

## FAQ
### How fast is `speare`?
I haven't benchmarked the newest versions yet, nor done any performance optimizations as well. There is an overhead due to boxing when sending messages, but overall it is pretty fast. Benchmarks TBD and added here in the future.

### Is it production ready?
Not yet! But soon :-)

### Why should I use this as opposed to the other 3 billion Rust actor frameworks?
I built `speare` because I wanted a very minimal actor framework providing just enough to abstract over tokio green threads and channels without introducing a lot of boilerplate or a lot of new concepts to my co-workers. You should use `speare` if you like its design, if not then don't :-)

### Can I contribute?
Sure! There are only two rules to contribute:

- Be respectful
- Don't be an asshole

Keep in mind I want to keep this very minimalistic, but am very open to suggestions and new ideas :)
