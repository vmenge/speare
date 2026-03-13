# Service Discovery

`speare` has a global actor registry shared across the entire `Node` tree. Actors can register themselves at spawn time and be looked up by other actors without needing direct `Handle` references. This enables loose coupling between actors that don't know about each other at compile time.

## Registering by Type

Use `spawn_registered()` to register an actor under its type name. This enforces a singleton pattern -- only one actor of each type can be registered at a time.

```rust,ignore
ctx.actor::<Logger>(()).spawn_registered()?;
```

The return type is `Result<Handle<Msg>, RegistryError>`. If a `Logger` is already registered, this returns `Err(RegistryError::NameTaken(...))`.

Here is a full `Logger` actor that we will register as a singleton:

```rust,ignore
use speare::*;

struct Logger;

enum LogMsg {
    Info(String),
    Error(String),
}

impl Actor for Logger {
    type Props = ();
    type Msg = LogMsg;
    type Err = ();

    async fn init(_ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        Ok(Logger)
    }

    async fn handle(&mut self, msg: Self::Msg, _ctx: &mut Ctx<Self>) -> Result<(), Self::Err> {
        match msg {
            LogMsg::Info(m) => println!("[INFO] {m}"),
            LogMsg::Error(m) => eprintln!("[ERROR] {m}"),
        }

        Ok(())
    }
}
```

A parent actor spawns and registers it during init:

```rust,ignore
// In some parent actor's init:
ctx.actor::<Logger>(()).spawn_registered()?;
```

## Registering by Name

Use `spawn_named()` to register an actor under a custom string key. This allows multiple actors of the same type to coexist in the registry with different names.

```rust,ignore
ctx.actor::<Worker>(props1).spawn_named("worker-1")?;
ctx.actor::<Worker>(props2).spawn_named("worker-2")?;
```

Like `spawn_registered()`, this returns `Result<Handle<Msg>, RegistryError>` and fails with `Err(RegistryError::NameTaken(...))` if the name is already taken.

## Looking Up Actors

Once registered, any actor in the tree can look up a handle without having received one directly.

**By type:**

```rust,ignore
let logger: Handle<LogMsg> = ctx.get_handle_for::<Logger>()?;
```

`get_handle_for::<A>()` returns `Result<Handle<A::Msg>, RegistryError>`. It infers the message type from the actor type.

**By name:**

```rust,ignore
let w1: Handle<WorkerMsg> = ctx.get_handle::<WorkerMsg>("worker-1")?;
```

`get_handle::<Msg>(name)` returns `Result<Handle<Msg>, RegistryError>`. You must specify the message type explicitly, since the registry only knows the name string.

Both methods return `Err(RegistryError::NotFound(...))` if no actor is registered under that type or name.

## Sending to Registered Actors

For fire-and-forget messages, `Ctx` provides convenience methods that combine lookup and send in one call. No need to store the `Handle`.

**By type:**

```rust,ignore
ctx.send::<Logger>(LogMsg::Info("System started".into()))?;
```

**By name:**

```rust,ignore
ctx.send_to::<WorkerMsg>("worker-1", WorkerMsg::Process(data))?;
```

Both return `Result<(), RegistryError>`, failing with `NotFound` if the actor is not in the registry.

If you need to send multiple messages, grab the handle once and reuse it:

```rust,ignore
let logger = ctx.get_handle_for::<Logger>()?;
logger.send(LogMsg::Info("Starting up".into()));
logger.send(LogMsg::Error("Something went wrong".into()));
```

## Putting It Together

Any actor anywhere in the tree can send to the registered `Logger` without ever receiving its handle directly:

```rust,ignore
// Any actor anywhere in the tree can send logs:
ctx.send::<Logger>(LogMsg::Info("System started".into()))?;

// Or get the handle for repeated use:
let logger = ctx.get_handle_for::<Logger>()?;
logger.send(LogMsg::Error("Something went wrong".into()));
```

Named workers follow the same pattern:

```rust,ignore
// Spawn workers with unique names:
ctx.actor::<Worker>(props1).spawn_named("worker-1")?;
ctx.actor::<Worker>(props2).spawn_named("worker-2")?;

// Send by name:
ctx.send_to::<WorkerMsg>("worker-1", WorkerMsg::Process(data))?;

// Or get the handle:
let w2 = ctx.get_handle::<WorkerMsg>("worker-2")?;
w2.send(WorkerMsg::Process(more_data));
```

## Auto-Deregistration

When an actor stops -- whether by error, parent shutdown, or an explicit `handle.stop()` call -- it is automatically removed from the registry. Subsequent lookups for that type or name will return `Err(RegistryError::NotFound(...))`.

This means the registry always reflects which actors are actually alive. You do not need to manually clean up entries.

## Error Handling

All registry operations return `Result<_, RegistryError>`. The enum has three variants:

```rust,ignore
pub enum RegistryError {
    /// Tried to register a name that's already in use.
    NameTaken(String),
    /// No actor registered under that name or type.
    NotFound(String),
    /// Internal lock poisoned (rare -- indicates a panic elsewhere).
    PoisonErr,
}
```

`RegistryError` implements `Display` and `Error`, so it works with `?` and standard error handling.

In practice, `NameTaken` and `NotFound` are the variants you will encounter. `PoisonErr` only occurs if another thread panicked while holding the registry lock, which should not happen under normal operation.
