# speare

[![crates.io](https://img.shields.io/crates/v/speare.svg)](https://crates.io/crates/speare)
[![docs.rs](https://docs.rs/speare/badge.svg)](https://docs.rs/speare)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

`speare` is a minimalistic actor framework. A thin abstraction over tokio tasks and flume channels with supervision inspired by Akka.NET.


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