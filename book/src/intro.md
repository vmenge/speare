# Introduction
This guide introduces `speare`, covering `Process` creation, message handling, and essential library features. The objective is to provide clear, concise instructions for effectively utilizing `speare` in your Rust projects.

## What is `speare`?
`speare` is a Rust library designed to simplify the process of actor-based concurrency. It provides an abstraction over [tokio green threads](https://tokio.rs/tokio/tutorial/spawning#tasks) and [flume channels](https://github.com/zesterer/flume), allowing for easier management of tokio threads and efficient message passing between them. With `speare`, each `Process` owns its data and lives on its own green thread, reducing the risks of deadlocks and encouraging a modular design. No more `Arc<Mutex<T>>` everywhere :)