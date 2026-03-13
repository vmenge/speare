use crate::{Actor, Ctx, Handle, ProcAction, ProcMsg, SpawnBuilder, Supervision};
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

    /// Stops all children. (Drop impl is fire and forget)
    pub async fn shutdown(&mut self) {
        self.ctx.stop_children().await;
    }
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

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
