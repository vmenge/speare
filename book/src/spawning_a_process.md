# Spawning a Process
In `speare`, a process represents an isolated unit of execution, similar to an actor in actor-based concurrency models. Each process maintains its state and communicates with other processes via message passing.

## Creating a Process
In `speare`, a process is defined as a struct, which becomes a process through the `#[process]` macro on its implementation block. This macro automatically implements the `Process` trait for your struct, transforming it into a functional process that can be managed within the `speare` framework.

```rust
use speare::*;

struct MyProcess {
    // Process state
}

#[process]
impl MyProcess {
    // Process methods
}

```

By using the `#[process]` macro, `MyProcess` now has the capabilities of a `Process`, including lifecycle management and message handling, as defined in the `Process` trait.

## Spawning the Process
To bring your process to life in `speare`, use a `Node` to spawn it. This action initiates the process and returns a `Pid<P>`, where `P` is the type of your process that implemented the `Process` trait.

```rust
#[tokio::main]
async fn main() {
    let node = Node::default();
    let process_id: Pid<MyProcess> = node.spawn(MyProcess { /* initial state */ }).await;
}
```

The `Pid<P>` serves as a unique identifier and a handle for the process, allowing for message passing and process management within the `speare` ecosystem.

## Nodes
A `Node` provides methods to spawn, send messages to, and terminate processes. It is essentially the environment in which a `Process` is spawned. You can create multiple nodes in your application to isolate processes from each other, but in most cases you'll need only one throughout your whole program.

## Example: Counter Process
Here’s a simple example of a counter `Process` that increments its state with each message received.

```rust
use speare::*;

struct IncreaseBy(u64);

struct Counter {
    count: u64,
}

#[process]
impl Counter {
    #[handler]
    async fn increment(&mut self, msg: IncreaseBy) -> Reply<(),()> {
        self.count += msg.0;
        reply(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    let counter_pid = node.spawn(Counter { count: 0 }).await;
    node.tell(&counter_pid, IncreaseBy(5)).await;
}

```