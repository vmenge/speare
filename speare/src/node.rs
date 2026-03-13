use crate::{Actor, Ctx, Handle, ProcAction, ProcMsg, RegistryError, SpawnBuilder, Supervision};
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
        };

        Self { ctx }
    }

    pub fn actor<'a, Child>(&'a mut self, props: Child::Props) -> SpawnBuilder<'a, Node, Child>
    where
        Child: Actor,
    {
        SpawnBuilder::new(&mut self.ctx, props)
    }

    pub fn get_handle_for<A: Actor>(&self) -> Result<Handle<A::Msg>, RegistryError> {
        self.ctx.get_handle_for::<A>()
    }

    pub fn get_handle<Msg: Send + 'static>(&self, name: &str) -> Result<Handle<Msg>, RegistryError> {
        self.ctx.get_handle(name)
    }

    pub fn send<A: Actor>(&self, msg: impl Into<A::Msg>) -> Result<(), RegistryError> {
        self.ctx.send::<A>(msg)
    }

    pub fn send_to<Msg: Send + 'static>(
        &self,
        name: &str,
        msg: impl Into<Msg>,
    ) -> Result<(), RegistryError> {
        self.ctx.send_to(name, msg)
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
