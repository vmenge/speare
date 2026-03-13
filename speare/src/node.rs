use crate::{
    Actor, Ctx, Handle, ProcAction, ProcMsg, PubSubError, RegistryError, SpawnBuilder, Supervision,
};
use std::collections::HashMap;
use tokio::task::{self, JoinSet};

pub struct Node {
    ctx: Ctx<Node>,
}

impl Actor for Node {
    type Props = ();
    type Msg = ();
    type Err = ();

    async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
        unreachable!("how did you get here")
    }
}

impl Node {
    /// Creates a new `Node`, the root of an actor supervision tree.
    ///
    /// A `Node` is the entry point for spawning actors. It holds shared state
    /// (registry, pub/sub bus) that all actors in the tree can access.
    pub fn new() -> Self {
        let (msg_tx, msg_rx) = flume::unbounded(); // child
        let (proc_msg_tx, proc_msg_rx) = flume::unbounded(); // child

        let handle = Handle {
            msg_tx,
            proc_msg_tx,
        };

        let ctx = Ctx {
            id: 0,
            props: (),
            handle,
            msg_rx,
            parent_proc_msg_tx: None,
            proc_msg_rx,
            children_proc_msg_tx: HashMap::new(),
            total_children: 0,
            supervision: Supervision::Stop,
            restarts: 0,
            tasks: JoinSet::new(),
            registry_key: None,
            registry: Default::default(),
            pubsub: Default::default(),
            subscription_ids: Vec::new(),
        };

        Self { ctx }
    }

    /// Creates a [`SpawnBuilder`] for spawning a top-level [`Actor`]. The actor type
    /// is passed as a generic parameter and its props as the argument.
    ///
    /// # Example
    /// ```ignore
    /// let handle = node.actor::<MyActor>(my_props)
    ///     .supervision(Supervision::Stop)
    ///     .spawn();
    /// ```
    pub fn actor<'a, Child>(&'a mut self, props: Child::Props) -> SpawnBuilder<'a, Node, Child>
    where
        Child: Actor,
    {
        SpawnBuilder::new(&mut self.ctx, props)
    }

    /// Looks up a registered [`Actor`]'s [`Handle`] by its type. The actor must have been
    /// spawned with [`SpawnBuilder::spawn_registered`].
    ///
    /// # Example
    /// ```ignore
    /// let logger = node.get_handle_for::<Logger>()?;
    /// logger.send(LogMsg::Info("hello".into()));
    /// ```
    pub fn get_handle_for<A: Actor>(&self) -> Result<Handle<A::Msg>, RegistryError> {
        self.ctx.get_handle_for::<A>()
    }

    /// Looks up a registered [`Actor`]'s [`Handle`] by name. The actor must have been
    /// spawned with [`SpawnBuilder::spawn_named`].
    ///
    /// # Example
    /// ```ignore
    /// let worker = node.get_handle::<WorkerMsg>("worker-1")?;
    /// worker.send(WorkerMsg::Start);
    /// ```
    pub fn get_handle<Msg: Send + 'static>(
        &self,
        name: &str,
    ) -> Result<Handle<Msg>, RegistryError> {
        self.ctx.get_handle(name)
    }

    /// Sends a message to a registered [`Actor`] looked up by type.
    ///
    /// # Example
    /// ```ignore
    /// node.send::<MetricsCollector>(MetricsMsg::RecordLatency(42))?;
    /// ```
    pub fn send<A: Actor>(&self, msg: impl Into<A::Msg>) -> Result<(), RegistryError> {
        self.ctx.send::<A>(msg)
    }

    /// Sends a message to a registered [`Actor`] looked up by name.
    ///
    /// # Example
    /// ```ignore
    /// node.send_to("worker-1", WorkerMsg::Start)?;
    /// ```
    pub fn send_to<Msg: Send + 'static>(
        &self,
        name: &str,
        msg: impl Into<Msg>,
    ) -> Result<(), RegistryError> {
        self.ctx.send_to(name, msg)
    }

    /// Publishes a message to all subscribers of a topic. The message is cloned
    /// for each subscriber.
    ///
    /// Publishing to a topic with no subscribers is a no-op and returns `Ok(())`.
    /// Returns `PubSubError::TypeMismatch` if the topic exists with a different type.
    pub fn publish<T: Send + 'static>(&self, topic: &str, msg: T) -> Result<(), PubSubError> {
        self.ctx.publish(topic, msg)
    }

    /// Stops all children and waits for each to fully terminate before returning.
    ///
    /// Prefer calling this over letting the `Node` be dropped. The `Drop` implementation
    /// sends stop signals to children but spawns a background tokio task to await their
    /// acknowledgments. If the tokio runtime is shutting down at the same time (e.g. the
    /// end of `#[tokio::main]`), that background task may never execute, leaving children
    /// in a partially-stopped state.
    pub async fn shutdown(&mut self) {
        self.ctx.stop_children().await;
    }
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

/// Sends stop signals to all children and spawns a background task to await their
/// acknowledgments. Because the awaiting happens in a spawned tokio task, the children
/// may **not** fully shut down if the tokio runtime is also shutting down (e.g. at the
/// end of `#[tokio::main]`). For guaranteed cleanup, call [`Node::shutdown`] before
/// dropping the `Node`.
impl Drop for Node {
    fn drop(&mut self) {
        let mut acks = Vec::with_capacity(self.ctx.total_children as usize);
        for child in self.ctx.children_proc_msg_tx.values() {
            let (ack_tx, ack_rx) = flume::unbounded();
            let _ = child.send(ProcMsg::FromParent(ProcAction::Stop(ack_tx)));
            acks.push(ack_rx);
        }

        task::spawn(async {
            for ack in acks {
                let _ = ack.recv_async().await;
            }
        });
    }
}
