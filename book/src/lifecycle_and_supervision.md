# Lifecycle & Supervision

Every actor in `speare` follows a predictable lifecycle, and when things go wrong, its parent decides what happens next. This chapter covers both: how actors live and die, how to build parent-child hierarchies, and how to configure supervision strategies for automatic recovery.

## Actor Lifecycle

An actor goes through the following stages:

```text
spawn → init() → [process messages via handle()] → exit()
```

- `init()` constructs the actor. This is where you set up initial state, spawn children, or open connections.
- `handle()` is called once per incoming message, for as long as the actor is alive.
- `exit()` runs when the actor stops for any reason -- error, manual stop, or parent shutdown.

On restart, `exit()` is called first, then `init()` runs again. The `Handle` stays the same across restarts, so any code holding a reference to it does not need to update.

### When does an actor stop?

An actor lives until one of these happens:

- **Manual stop** -- someone calls `handle.stop()`.
- **Parent stops** -- when a parent actor stops, all of its children are stopped first.
- **Unrecoverable error** -- the actor returns an `Err` and its supervision strategy is `Supervision::Stop` (or it has exhausted its restart limit).

### ExitReason

The `exit()` callback receives an `ExitReason` so you can react accordingly:

```rust,ignore
pub enum ExitReason<P: Actor> {
    Handle,       // stopped manually via handle.stop()
    Parent,       // parent actor stopped this child
    Err(P::Err),  // actor's code returned an error
}
```

```rust,ignore
async fn exit(this: Option<Self>, reason: ExitReason<Self>, _ctx: &mut Ctx<Self>) {
    match reason {
        ExitReason::Handle => println!("stopped by handle"),
        ExitReason::Parent => println!("parent shut us down"),
        ExitReason::Err(e) => println!("failed with error: {e:?}"),
    }
}
```

Note that `this` is `Option<Self>` -- it will be `None` if `init()` itself failed.

## Parent-Child Actors

Inside an actor's `init` or `handle`, you can spawn child actors via `ctx.actor::<Child>(props).spawn()`. The child is supervised by the actor that spawned it.

```rust,ignore
use speare::*;

struct Worker;
enum WorkerMsg { Process(String) }

impl Actor for Worker {
    type Props = ();
    type Msg = WorkerMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        println!("Worker started");
        Ok(Worker)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            WorkerMsg::Process(job) => println!("Processing: {job}"),
        }

        Ok(())
    }

    async fn exit(_this: Option<Self>, _reason: ExitReason<Self>, _ctx: &mut Ctx<Self>) {
        println!("Worker stopped");
    }
}

struct Manager {
    worker: Handle<WorkerMsg>,
}

enum ManagerMsg { Dispatch(String) }

impl Actor for Manager {
    type Props = ();
    type Msg = ManagerMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        let worker = ctx.actor::<Worker>(()).spawn();
        Ok(Manager { worker })
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            ManagerMsg::Dispatch(job) => self.worker.send(WorkerMsg::Process(job)),
        }

        Ok(())
    }
}

// When the Manager is stopped, the Worker is automatically stopped too.
```

When a parent stops, all of its children are stopped first. This cascades down the entire tree -- if the Manager has workers, and those workers have their own children, everything shuts down in order from the leaves up.

You can also stop all children manually without stopping the parent:

```rust,ignore
ctx.stop_children().await;
```

This stops every child actor the current actor has spawned and waits for each to fully terminate before returning.

Similarly, you can restart all children at once:

```rust,ignore
ctx.restart_children();
```

This sends a restart signal to every child. Each child will re-run `exit()` then `init()`, resetting its state while keeping its `Handle` valid. Unlike `stop_children`, this is fire-and-forget -- it does not wait for the children to finish restarting. The restart bypasses each child's supervision strategy (it always restarts, regardless of `Supervision::Stop` or restart limits).

## Supervision Strategies

When you spawn a child actor, you can configure what should happen if it returns an error. Set the strategy with `.supervision()` on the spawn builder:

```rust,ignore
let handle = ctx.actor::<Child>(props)
    .supervision(strategy)
    .spawn();
```

There are three strategies:

### Supervision::Stop (default)

The actor terminates on error. `exit()` is called, and the actor is done.

```rust,ignore
ctx.actor::<Worker>(()).supervision(Supervision::Stop).spawn();
```

This is the default -- if you call `.spawn()` without `.supervision()`, you get `Stop`.

### Supervision::Resume

The actor ignores the error and continues processing the next message. The actor is **not** restarted -- `exit()` and `init()` are not called. The actor keeps its current state and moves on.

```rust,ignore
ctx.actor::<Worker>(()).supervision(Supervision::Resume).spawn();
```

### Supervision::Restart

The actor is restarted: `exit()` is called, then `init()` runs again. The `Handle` stays the same, so senders do not need to update their references.

```rust,ignore
ctx.actor::<Worker>(())
    .supervision(Supervision::Restart {
        max: Limit::Amount(3),
        backoff: Backoff::None,
    })
    .spawn();
```

## Restart Limits

The `max` field on `Supervision::Restart` controls how many times the actor can be restarted before giving up:

- `Limit::None` -- unlimited restarts. The actor will be restarted every time it errors, forever. Use with caution.
- `Limit::Amount(n)` -- restart at most `n` times. Once the limit is reached, the actor terminates for real, just as if the strategy were `Supervision::Stop`.

```rust,ignore
// Restart up to 5 times, then give up
Supervision::Restart {
    max: Limit::Amount(5),
    backoff: Backoff::None,
}

// Restart forever
Supervision::Restart {
    max: Limit::None,
    backoff: Backoff::None,
}
```

## Backoff Strategies

The `backoff` field on `Supervision::Restart` controls how long to wait between restart attempts:

- `Backoff::None` -- restart immediately, no delay.
- `Backoff::Static(Duration)` -- fixed delay between each restart. 
- `Backoff::Incremental { min, max, step }` -- delay increases linearly by `step` per restart, clamped between `min` and `max`.

Here is a complete example. First, define an actor that always fails:

```rust,ignore
use speare::*;
use std::time::Duration;

struct Flaky;
enum FlakyMsg { DoWork }

#[derive(Debug)]
struct FlakyErr;

impl Actor for Flaky {
    type Props = ();
    type Msg = FlakyMsg;
    type Err = FlakyErr;

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        println!("Flaky actor (re)starting");
        Ok(Flaky)
    }

    async fn handle(&mut self, _msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        Err(FlakyErr) // always fails
    }
}
```

Then spawn it with a restart strategy and incremental backoff:

```rust,ignore
// In a parent actor's init():
let flaky = ctx.actor::<Flaky>(())
    .supervision(Supervision::Restart {
        max: Limit::Amount(3),
        backoff: Backoff::Incremental {
            min: Duration::from_millis(100),
            max: Duration::from_secs(5),
            step: Duration::from_millis(500),
        },
    })
    .spawn();
```

Each time `Flaky` errors, it will be restarted after a growing delay: 500ms, then 1000ms, then 1500ms (clamped between 100ms and 5s). After the third restart, it terminates permanently.

## Watching Children

Sometimes a parent needs to know when a child has permanently failed. The `.watch()` method on the spawn builder lets you register a callback that fires when the child terminates due to an unrecoverable error.

Watch fires when:
- The strategy is `Supervision::Stop` and the child errors.
- The strategy is `Supervision::Restart` and the child exhausts all its allowed restarts.

Watch does **not** fire when:
- The child is restarted successfully (it has retries remaining).
- The child is stopped manually via `handle.stop()`.
- The strategy is `Supervision::Resume`.

The callback maps the child's error into a message for the parent:

```rust,ignore
use speare::*;

struct Supervisor;

enum SupervisorMsg {
    Start,
    WorkerDied(String),
}

impl Actor for Supervisor {
    type Props = ();
    type Msg = SupervisorMsg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        // Spawn a worker with restart supervision and a watch callback
        ctx.actor::<Flaky>(())
            .supervision(Supervision::Restart {
                max: Limit::Amount(3),
                backoff: Backoff::None,
            })
            .watch(|err| SupervisorMsg::WorkerDied(format!("{err:?}")))
            .spawn();

        Ok(Supervisor)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            SupervisorMsg::Start => { /* ... */ }
            SupervisorMsg::WorkerDied(reason) => {
                println!("Worker permanently failed: {reason}");
                // Could spawn a replacement, alert, etc.
            }
        }

        Ok(())
    }
}
```

After `Flaky` fails 3 times and exhausts its restart limit, the watch callback fires and sends `SupervisorMsg::WorkerDied` to the `Supervisor`. The parent can then decide what to do -- spawn a replacement, escalate, log the failure, or shut itself down.

## Replicating BEAM Supervision Strategies

If you are coming from Elixir or Erlang, you may be familiar with three supervisor strategies:

- **one_for_one** -- if a child fails, only that child is restarted.
- **one_for_all** -- if any child fails, all children are stopped and restarted.
- **rest_for_one** -- if a foundational child fails, all children that depend on it are restarted too.

In speare, supervision is configured per-child rather than per-supervisor. `Supervision::Restart` gives you `one_for_one` out of the box. The other two can be built by combining `.watch()` with `stop_children()` or `restart_children()`.

### one_for_one

This is speare's default behavior. Each child gets its own `Supervision::Restart`:

```rust,ignore
ctx.actor::<WorkerA>(())
    .supervision(Supervision::Restart {
        max: Limit::Amount(3),
        backoff: Backoff::None,
    })
    .spawn();

ctx.actor::<WorkerB>(())
    .supervision(Supervision::Restart {
        max: Limit::Amount(3),
        backoff: Backoff::None,
    })
    .spawn();
```

If WorkerA fails, only WorkerA is restarted. WorkerB is unaffected.

### one_for_all

Use `.watch()` to detect when any child permanently fails, then stop all remaining children and re-spawn the entire group:

```rust,ignore
struct Supervisor;

enum Msg {
    ChildFailed,
}

impl Actor for Supervisor {
    type Props = ();
    type Msg = Msg;
    type Err = ();

    async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        spawn_all(ctx);
        Ok(Supervisor)
    }

    async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            Msg::ChildFailed => {
                ctx.stop_children().await;
                spawn_all(ctx);
            }
        }
        Ok(())
    }
}

fn spawn_all(ctx: &mut Ctx<Supervisor>) {
    ctx.actor::<WorkerA>(())
        .watch(|_| Msg::ChildFailed)
        .spawn();
    ctx.actor::<WorkerB>(())
        .watch(|_| Msg::ChildFailed)
        .spawn();
    ctx.actor::<WorkerC>(())
        .watch(|_| Msg::ChildFailed)
        .spawn();
}
```

When any worker terminates for good, the supervisor stops all remaining children and re-spawns the entire group. The `stop_children` call does not trigger `.watch()` on the surviving children -- watch only fires on error termination -- so there are no spurious `ChildFailed` messages from the teardown.

You can also give each child individual restart attempts before escalating to the group restart. Just add a supervision strategy alongside the watch:

```rust,ignore
ctx.actor::<WorkerA>(())
    .supervision(Supervision::Restart {
        max: Limit::Amount(3),
        backoff: Backoff::None,
    })
    .watch(|_| Msg::ChildFailed)
    .spawn();
```

Now WorkerA gets 3 restart attempts on its own. Only if it exhausts those does the watch fire and trigger the full group restart.

### rest_for_one

Watch the foundational actor that others depend on. If it dies, the children that depend on it need to restart too. Suppose WorkerB and WorkerC both depend on WorkerA:

```rust,ignore
enum Msg {
    WorkerAFailed,
}

async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ctx.actor::<WorkerA>(())
        .watch(|_| Msg::WorkerAFailed)
        .spawn();
    ctx.actor::<WorkerB>(()).spawn();
    ctx.actor::<WorkerC>(()).spawn();

    Ok(Supervisor)
}

async fn handle(&mut self, msg: Self::Msg, ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
    match msg {
        Msg::WorkerAFailed => {
            // Option 1: tear down everything and start fresh
            ctx.stop_children().await;
            // re-spawn A, B, C...

            // Option 2: keep B and C's mailbox
            ctx.restart_children(); // restarts the surviving B and C
            // re-spawn A (since it's already dead)
            ctx.actor::<WorkerA>(())
                .watch(|_| Msg::WorkerAFailed)
                .spawn();
        }
    }
    Ok(())
}
```

Option 1 is the clean slate -- stop everything and re-spawn in order. Option 2 preserves the mailbox on the surviving children by restarting them (re-running their `init` with the same props) while only re-spawning the dead actor.
