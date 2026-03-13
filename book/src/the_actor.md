# The Actor

An actor is a struct that lives on its own tokio task, owns its data, and processes messages one at a time. There is no shared mutable state between actors -- they communicate exclusively through message passing. `speare` manages the lifecycle, communication, and supervision of your actors so you can focus on business logic rather than concurrency plumbing.

## A Minimal Actor

Here is a complete `Greeter` actor that prints a hello message for each name it receives:

```rust,ignore
use speare::*;

struct Greeter;

enum GreeterMsg {
    Greet(String),
}

impl Actor for Greeter {
    type Props = ();
    type Msg = GreeterMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Greeter)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            GreeterMsg::Greet(name) => println!("Hello, {name}!"),
        }

        Ok(())
    }
}
```

The `Actor` trait is the core abstraction in `speare`. You implement it on a struct, define a few associated types, and provide at minimum an `init` function. That is enough to have a working actor.

`init` constructs the actor's state. It runs when the actor is first spawned and again on every restart. `handle` is called once for each incoming message, and messages are always processed sequentially -- there is no concurrent access to `&mut self`.

## Associated Types

The `Actor` trait requires three associated types.

### `Props: Send + 'static`

`Props` is immutable configuration or dependencies your actor needs. It is passed in at spawn time and remains available across restarts via `ctx.props()`. When your actor does not need configuration, use `()`.

### `Msg: Send + 'static`

`Msg` is the message type your actor processes. Typically this is an enum with one variant per command the actor understands.

### `Err: Send + Sync + 'static`

`Err` is the error type. Returning `Err` from `init` or `handle` triggers the supervision strategy configured by the parent.

## Trait Functions

### `init` (required)

```rust,ignore
async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err>;
```

The only required function. Called when the actor is first spawned and again on every restart. Its job is to construct and return the actor's initial state. You have access to the full `Ctx` here, so you can read props, clear stale messages from a previous life, or even spawn child actors.

If `init` returns `Err`, the actor never enters the message loop. The supervision strategy is consulted and, if applicable, `init` will be called again on restart.

### `handle` (optional)

```rust,ignore
async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err>;
```

Called once for every message the actor receives. Messages are processed sequentially -- the next message is not dequeued until the current `handle` call completes. This means you never need a lock on the actor's state.

Returning `Err` triggers the supervision strategy. With `Supervision::Stop`, the actor shuts down. With `Supervision::Restart`, it re-runs `init`. With `Supervision::Resume`, it simply continues to the next message.

### `exit` (optional)

```rust,ignore
async fn exit(this: Option<Self>, reason: ExitReason<Self>, ctx: &mut Ctx<Self>);
```

A cleanup hook called when the actor stops, restarts, or fails to initialize. The `this` parameter is `Some` if the actor was successfully constructed and `None` if `init` itself failed.

`ExitReason` tells you why the actor is exiting:

- `ExitReason::Handle` -- the actor was stopped manually via `handle.stop()`.
- `ExitReason::Parent` -- the parent actor stopped or restarted this actor.
- `ExitReason::Err(e)` -- the actor returned an error from `init` or `handle`.

### `sources` (optional)

```rust,ignore
async fn sources(&self, ctx: &Ctx<Self>) -> Result<impl Sources<Self>, Self::Err>;
```

Sets up additional message sources such as streams and intervals. This is covered in [Concurrency Patterns](concurrency_patterns.md).

## The Context (`Ctx`)

Every trait function receives a reference to `Ctx<Self>`, which provides access to the actor's environment. Some commonly used methods are:

### `ctx.props()`

Returns `&Props`. This is the immutable configuration passed at spawn time. It persists across restarts, so you can always rely on it to rebuild your actor's state inside `init`.

```rust,ignore
async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    Ok(Counter {
        count: ctx.props().initial_count,
    })
}
```

### `ctx.this()`

Returns `&Handle<Self::Msg>`, a handle to the current actor. This is useful for sending messages to yourself (for example, scheduling a delayed self-message) or for passing your handle to a child actor so it can communicate back.

```rust,ignore
async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    let my_handle = ctx.this().clone();
    // pass my_handle to a child, or send yourself a message:
    my_handle.send(MyMsg::Bootstrap);
    Ok(MyActor)
}
```

### `ctx.actor()`

Spawns a child actor supervised by the current actor. The child type is passed as a generic parameter and its props as the argument. Returns a `SpawnBuilder` that lets you configure a supervision strategy before calling `.spawn()`, which returns a `Handle<Child::Msg>` you can use to send messages to the child.

Children are automatically stopped when the parent stops.

```rust,ignore
async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    let worker_handle = ctx.actor::<Worker>(WorkerProps { id: 1 })
        .supervision(Supervision::Restart {
            max: Limit::Amount(3),
            backoff: Backoff::None,
        })
        .spawn();

    Ok(MyActor { worker_handle })
}
```

### `ctx.clear_mailbox()`

Drains all pending messages from the actor's mailbox. This is most useful inside `init` during a restart -- if the actor crashed because of a bad message, you may want to throw away whatever was queued up and start fresh.

```rust,ignore
async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ctx.clear_mailbox();
    Ok(MyActor::default())
}
```

## Full Example

The following example ties everything together. A `Counter` actor uses custom `CounterProps` for configuration, a `CounterErr` for domain errors, and implements `init`, `handle`, and `exit`.

```rust,ignore
use speare::*;

struct Counter {
    count: u32,
}

struct CounterProps {
    initial_count: u32,
    max_count: u32,
}

enum CounterMsg {
    Add(u32),
    Subtract(u32),
}

#[derive(Debug)]
enum CounterErr {
    MaxCountExceeded,
}

impl Actor for Counter {
    type Props = CounterProps;
    type Msg = CounterMsg;
    type Err = CounterErr;

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        println!("Counter starting with {}", ctx.props().initial_count);
        Ok(Counter {
            count: ctx.props().initial_count,
        })
    }

    async fn exit(this: Option<Self>, reason: ExitReason<Self>, _ctx: &mut Ctx<Self>) {
        match &reason {
            ExitReason::Err(e) => println!("Counter failed: {e:?}"),
            ExitReason::Handle => println!("Counter stopped manually"),
            ExitReason::Parent => println!("Counter stopped by parent"),
        }

        if let Some(counter) = this {
            println!("Final count was: {}", counter.count);
        }
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            CounterMsg::Add(n) => {
                self.count += n;
                if self.count > ctx.props().max_count {
                    return Err(CounterErr::MaxCountExceeded);
                }
            }

            CounterMsg::Subtract(n) => {
                self.count = self.count.saturating_sub(n);
            }
        }

        Ok(())
    }
}
```

`init` reads `initial_count` from props to set the starting state. `handle` performs arithmetic and returns `CounterErr::MaxCountExceeded` if the count goes over the configured `max_count` -- this will trigger whatever supervision strategy the parent has set. `exit` logs why the actor stopped and, if the actor was successfully created, prints the final count.

Props survive restarts. If this actor is supervised with `Supervision::Restart`, `init` will be called again with the same `CounterProps`, producing a fresh `Counter` with the original `initial_count`. The broken state is gone, but the configuration is preserved.
