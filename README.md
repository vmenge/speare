# speare
`speare` is a minimalistic actor framework that also has pub / sub capabities.

[![crates.io](https://img.shields.io/crates/v/speare.svg)](https://crates.io/crates/speare)
[![docs.rs](https://docs.rs/speare/badge.svg)](https://docs.rs/speare)

## Speare at a Glance
```rust
use speare::*;

struct Ping;
struct Pong;

struct ProcA {
    b_pid: Pid<ProcB>,
}

#[process]
impl ProcA {
    #[handler]
    async fn ping(&mut self, _msg: Ping, ctx: &Ctx<Self>) -> Reply<(), ()> {
        println!("ping!");
        ctx.tell(&self.b_pid, Pong).await;

        reply(())
    }
}

struct ProcB;

#[process]
impl ProcB {
    #[handler]
    async fn pong(&mut self, _msg: Pong) -> Reply<(), ()> {
        println!("pong!");

        reply(())
    }
}

#[tokio::main]
async fn main() {
    let node = Node::default();
    let b_pid = node.spawn(ProcB).await;
    let a_pid = node.spawn(ProcA { b_pid }).await;

    node.tell(&a_pid, Ping).await;

    // wait 1s otherwise program will end
    tokio::time::sleep(Duration::from_secs(1)).await;

    // ping!
    // pong!
}

```

### Features
- Minimalistic API
- [Fire and Forget, and Request / Response messages](https://vmenge.github.io/speare/handlers.html#message-passing-methods-tell-tell_in-and-ask)
- [Pub / Sub](https://vmenge.github.io/speare/pub_sub.html)
- [Lifecycle Management](https://vmenge.github.io/speare/lifecycle_management.html)
- [Deferred Replies](https://vmenge.github.io/speare/reply.html#deferring-replies)

### Documentation
[The Speare Book](https://vmenge.github.io/speare/) takes around 20min to read through and will teach you all you need to know to use `speare` effectively.

## Why `speare`?
`speare` is a minimal abstraction layer over [tokio green threads](https://tokio.rs/tokio/tutorial/spawning#tasks) and [flume channels](https://github.com/zesterer/flume), offering functionality to manage these threads, and pass messages between these in a more practical manner. The question instead should be: *"why message passing (channels) instead of sharing state (e.g. `Arc<Mutex<T>>`)?"*

- **Easier reasoning**: With message passing, each piece of data is owned by a single thread at a time, making the flow of data and control easier to reason about.
- **Deadlock Prevention**: Shared state with locks (like mutexes) can lead to deadlocks if not managed carefully. Message passing, especially in Rust, is less prone to deadlocks as it doesn’t involve traditional locking mechanisms.
- **Encouragement of Decoupled Design**: Message passing promotes a more modular, decoupled design where components communicate through well-defined interfaces (channels), enhancing maintainability and scalability of the code.

## FAQ
### How fast is `speare`?
I haven't benchmarked the newest versions yet, nor done any performance optimizations as well. There is an overhead due to boxing when sending messages, but overall it is pretty fast. Benchmarks TBD and added here in the future.

### Is it production ready?
The API probably wont change very much from here on out, but there are probably some bugs around so feel free to create issues on the repo. I haven't deployed it to production yet, but I should soon after I've implemented and tested it thoroughly on a production codebase.

### Why should I use this as opposed to the other 3 billion Rust actor frameworks?
I built `speare` because I wanted a very minimal actor framework providing just enough to abstract over tokio green threads and channels without introducing a lot of boilerplate or a lot of new concepts to my co-workers. You should use `speare` if you like its design, if not then don't :-)

### Can I contribute?
Sure! There are only two rules to contribute:

- Be respectful
- Don't be an asshole

Keep in mind I want to keep this very minimalistic, but am very open to suggestions and new ideas :)