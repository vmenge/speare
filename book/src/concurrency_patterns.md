# Concurrency Patterns

This chapter covers two mechanisms for doing work alongside an actor's main message loop: **background tasks** and **message sources**.

## Background Tasks

`ctx.task(async { ... })` spawns an async task that runs concurrently with the actor. The task returns `Result<Msg, Err>`. When it completes:

- `Ok(msg)` -- the message is delivered to the actor's `handle()` method, just like any other message.
- `Err(e)` -- the actor's supervision strategy kicks in (restart, stop, or resume, depending on how the parent configured it).

Tasks are automatically aborted when the actor stops. You can spawn them from `init` or `handle`.

```rust,ignore
use speare::*;

struct Fetcher {
    results: Vec<String>,
}

enum FetcherMsg {
    Fetch(String),
    Fetched(String),
}

impl Actor for Fetcher {
    type Props = ();
    type Msg = FetcherMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Fetcher { results: vec![] })
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            FetcherMsg::Fetch(url) => {
                ctx.task(async move {
                    // some async work (e.g., HTTP request)
                    let body = format!("Response from {url}");
                    Ok(FetcherMsg::Fetched(body))
                });
            }

            FetcherMsg::Fetched(body) => {
                println!("Got: {body}");
                self.results.push(body);
            }
        }

        Ok(())
    }
}
```

The actor stays responsive to other messages while the task runs in the background. When the task completes, its `Ok` value is fed back into `handle()` as a regular message -- the actor processes it in turn, just like a message sent via `handle.send()`.

## Message Sources

The `sources` trait function declares additional message sources that run alongside the actor's main channel. It is called once, right after `init` succeeds. It returns a `SourceSet` -- a composable collection of intervals and streams. All sources and the main message channel are polled concurrently via `tokio::select!`, so the actor can react to whichever fires first.

```rust,ignore
async fn sources(&self, ctx: &Ctx<Self>) -> Result<impl Sources<Self>, Self::Err> {
    Ok(SourceSet::new()
        .interval(time::interval(Duration::from_secs(1)), || Msg::Tick))
}
```

You need to import `Sources` from speare. It is included when you use `use speare::*`.

## Intervals

`SourceSet::new().interval(tokio_interval, || Msg)` adds a recurring timer. Each time the interval fires, the closure produces a message that is delivered to `handle()`.

```rust,ignore
use speare::*;
use std::time::Duration;
use tokio::time;

struct Heartbeat {
    tick_count: u64,
}

enum HeartbeatMsg {
    Tick,
}

impl Actor for Heartbeat {
    type Props = ();
    type Msg = HeartbeatMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Heartbeat { tick_count: 0 })
    }

    async fn sources(&self, _ctx: &Ctx<Self>) -> Result<impl Sources<Self>, Self::Err> {
        Ok(SourceSet::new()
            .interval(time::interval(Duration::from_secs(1)), || HeartbeatMsg::Tick))
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            HeartbeatMsg::Tick => {
                self.tick_count += 1;
                println!("Heartbeat #{}", self.tick_count);
            }
        }

        Ok(())
    }
}
```

The interval is created with `tokio::time::interval`, which means the first tick fires immediately. If you want an initial delay, use `tokio::time::interval_at` instead.

## Streams

`SourceSet::new().stream(my_stream)` adds any type that implements `Stream<Item = Msg> + Send + Unpin + 'static`. This is the `Sources` trait from speare -- any stream whose items are the actor's message type works.

```rust,ignore
use speare::*;
use futures_core::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

// A simple stream wrapper around flume::Receiver
struct ReceiverStream<T> {
    rx: flume::Receiver<T>,
}

impl<T> Stream for ReceiverStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.try_recv() {
            Ok(item) => Poll::Ready(Some(item)),
            Err(flume::TryRecvError::Empty) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(flume::TryRecvError::Disconnected) => Poll::Ready(None),
        }
    }
}

impl<T> Unpin for ReceiverStream<T> {}
```

Any async data source -- a WebSocket connection, a file watcher, a Kafka consumer -- can be wrapped as a `Stream` and plugged into an actor this way. The actor does not need to know or care where the messages come from; they all arrive through `handle()`.

## Combining Sources

You can chain multiple `.interval()` and `.stream()` calls to build a `SourceSet` with several sources at once:

```rust,ignore
async fn sources(&self, _ctx: &Ctx<Self>) -> Result<impl Sources<Self>, Self::Err> {
    Ok(SourceSet::new()
        .interval(time::interval(Duration::from_secs(1)), || Msg::Tick)
        .stream(my_websocket_stream)
        .interval(time::interval(Duration::from_secs(30)), || Msg::Heartbeat))
}
```

All sources are merged and polled concurrently. Messages from intervals, streams, and regular `handle.send()` calls all arrive in `handle()` -- the actor processes them one at a time in the order they become ready.
