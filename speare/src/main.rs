use rs::{App, Pub, Receiver, Res, Result, Schedule::*, Sender, Sub};
use std::ops::Deref;

#[derive(Clone, Debug)]
struct MyEvent;

#[derive(Debug)]
struct Rero(String);

#[tokio::main]
async fn main() -> rs::Result<()> {
    App::new()
        .insert_resource(Rero(String::from("bla")))
        .register_channel::<MyEvent>()
        .register_broadcast::<MyEvent>(100)
        .add_system(Start, foo)
        .add_system(Task, handle_my_event)
        .add_system(Task, sub1)
        .add_system(Task, sub2)
        .add_system(Task, publ)
        .add_system(Shutdown, async || println!("shutting down!"))
        .add_system(Shutdown, async || println!("shutting down again!"))
        .start()
        .await?;

    Ok(())
}

async fn foo(tx: Sender<MyEvent>, rero: Res<Rero>) -> Result<()> {
    println!("sent event! rero: {}", rero.deref().0);
    tx.send(MyEvent).await?;

    Ok(())
}

async fn handle_my_event(rx: Receiver<MyEvent>) -> Result<()> {
    let evt = rx.recv().await?;
    println!("got event: {evt:?}");

    Ok(())
}

async fn sub1(mut rx: Sub<MyEvent>) {
    if let Ok(evt) = rx.recv().await {
        println!("got sub 1: {evt:?}");
    }
}

async fn sub2(mut rx: Sub<MyEvent>) {
    if let Ok(evt) = rx.recv().await {
        println!("got sub 2: {evt:?}");
    }
}

async fn publ(rx: Pub<MyEvent>) {
    if let Ok(subs) = rx.send(MyEvent) {
        println!("sent event to subscribers: {subs:?}");
    }
}
