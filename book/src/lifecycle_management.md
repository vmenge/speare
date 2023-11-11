# Lifecycle Management in Speare

In `speare`, understanding the lifecycle of a `Process` is key to effectively managing concurrency. The lifecycle stages include creation, initialization, execution, and termination, each offering opportunities for custom behavior.

## Process Lifecycle

1. **Creation**: A `Process` is created by the user.
2. **Spawning**: The `Process` is passed to the `spawn` function.
3. **Subscriptions**: Any [pub/sub event subscriptions](./pub_sub.md) are made.
4. **Task Initialization**: The `Process` is put into a `tokio::task`.
5. **Initialization**: `on_init` is called, allowing for startup behavior.
6. **Execution**: The `Process` runs, handling messages.
7. **Termination Signal**: Upon receiving an `ExitSignal<P>`, the `Process` begins termination.
8. **Cleanup**: `on_exit` is invoked for cleanup or final actions.
9. **Monitoring Responses**: Monitors listening for the `ExitSignal<P>` receive notifications.
10. **Task Completion**: The `tokio::task` concludes.

## Customizing Startup and Termination

Use `#[on_init]` and `#[on_exit]` macros to define custom behaviors at the start and end of a process's life.

```rust
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
    // ...other handlers...
}
```

## Terminating Processes

Use the `.exit()` function to gracefully terminate a process. It allows the process to finish its current message before exiting.

```rust
let node = Node::default();
let counter_pid = node.spawn(Counter::default()).await;
node.exit(&counter_pid, ExitReason::Shutdown).await;
```

## Monitoring Processes

You can monitor a process for its termination signal using `ExitSignal<P>`. Implement a `Handler` for `ExitSignal<P>` and use `.monitor()` on the process to receive notifications upon its exit.

```rust
struct ProcA;

#[process]
impl ProcA {}

struct ProcB;

#[process]
impl ProcB {
    #[on_init]
    async fn init(&mut self, ctx: &Ctx<Self>) {
        let a_pid = ctx.spawn(ProcA).await;
        ctx.monitor(&a_pid);
    }

    #[handler]
    async fn handle_proc_a_exit(&mut self, signal: ExitSignal<ProcA>) -> Reply<(), ()> {
        println!("ProcA exited!");
        reply(())
    }
}
```

Custom error types can be utilized during termination for more detailed exit signaling.

```rust
struct ProcA;

#[process(Error = String)]
impl ProcA {}

struct ProcB {
    a_pid: Pid<ProcA>
}

#[process]
impl ProcB {
    #[on_init]
    async fn init(&mut self, ctx: &Ctx<Self>) {
        ctx.monitor(&self.a_pid);
        ctx.exit(&a_pid, ExitReason::Err("something went wrong".to_string())).await;
    }

    #[handler]
    async fn handle_proc_a_exit(&mut self, signal: ExitSignal<ProcA>) -> Reply<(), ()> {
        let reason = match signal.reason() {
            ExitReason::Normal => "finishing running its tasks",
            ExitReason::Shutdown => "intentional interrupt by another process",
            ExitReason::Err(e) => e,
        };

        println!("ProcA exited due to: {}", reason);

        reply(())
    }
}

async fn run() {
    let node = Node::default();
    let a_pid = node.spawn(ProcA).await;
    node.spawn(ProcB { a_pid: a_pid.clone() }).await;
}
```
