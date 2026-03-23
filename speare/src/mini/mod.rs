use crate::{
    pubsub::{PubSub, Subscriber, TopicEntry},
    Backoff, Limit,
};
use flume::Receiver;
use std::{
    any::{Any, TypeId},
    cmp,
    collections::{hash_map::Entry, HashMap},
    future::Future,
    ops::Deref,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    task::{self, AbortHandle},
    time,
};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct TaskId(u64);

pub struct Ctx<Args = ()> {
    args: Args,
    pubsub: Arc<RwLock<PubSub>>,
    on_err: OnErr,
    children: HashMap<TaskId, AbortHandle>,
}

impl<Args> Deref for Ctx<Args> {
    type Target = Args;

    fn deref(&self) -> &Self::Target {
        &self.args
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SpeareErr {
    SubscribeTypeMismatch { topic: String },
    LockPoisonErr,
}

impl std::fmt::Display for SpeareErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpeareErr::SubscribeTypeMismatch { topic } => {
                write!(f, "pub/sub topic type mismatch for topic: {topic}")
            }

            SpeareErr::LockPoisonErr => write!(f, "internal RwLock poisoned"),
        }
    }
}

impl std::error::Error for SpeareErr {}

type Result<T, E = SpeareErr> = std::result::Result<T, E>;

impl<Args> Ctx<Args>
where
    Args: Send + 'static,
{
    /// Returns a flume::Receiver<T> to a topic.
    ///
    /// Returns `PubSubError::TypeMismatch` if the topic was already created with
    /// a different message type.
    pub fn subscribe<T>(&self, topic: &str) -> Result<Receiver<T>>
    where
        T: Clone + Send + 'static,
    {
        let mut bus = self.pubsub.write().map_err(|_| SpeareErr::LockPoisonErr)?;

        let type_id = TypeId::of::<T>();

        if let Some(entry) = bus.topics.get(topic) {
            if entry.type_id != type_id {
                return Err(SpeareErr::SubscribeTypeMismatch {
                    topic: topic.to_string(),
                });
            }
        }

        let sub_id = bus.next_sub_id;
        bus.next_sub_id += 1;

        let entry = bus
            .topics
            .entry(topic.to_string())
            .or_insert_with(|| TopicEntry {
                type_id,
                subscribers: Vec::new(),
            });

        let (msg_tx, msg_rx) = flume::unbounded();
        let send_fn = Box::new(move |any: &dyn Any| -> bool {
            if let Some(val) = any.downcast_ref::<T>() {
                msg_tx.send(val.clone()).is_ok()
            } else {
                false
            }
        });

        entry.subscribers.push(Subscriber {
            id: sub_id,
            send_fn,
        });

        Ok(msg_rx)
    }

    /// Publishes a message to all subscribers of a topic. The message is cloned
    /// for each subscriber.
    ///
    /// Publishing to a topic with no subscribers is a no-op and returns `Ok(())`.
    /// Returns `PubSubError::TypeMismatch` if the topic exists with a different type.
    pub fn publish<T>(&self, topic: &str, msg: T) -> Result<()>
    where
        T: Clone + Send + 'static,
    {
        let mut bus = self.pubsub.write().map_err(|_| SpeareErr::LockPoisonErr)?;

        let Some(entry) = bus.topics.get_mut(topic) else {
            return Ok(());
        };

        if entry.type_id != TypeId::of::<T>() {
            return Err(SpeareErr::SubscribeTypeMismatch {
                topic: topic.to_string(),
            });
        }

        let msg_any = &msg as &dyn Any;
        entry.subscribers.retain(|sub| (sub.send_fn)(msg_any));

        Ok(())
    }

    pub fn abort_child(&mut self, id: TaskId) -> bool {
        match self.children.entry(id) {
            Entry::Vacant(_) => false,
            Entry::Occupied(child) => {
                child.get().abort();
                child.remove();
                true
            }
        }
    }

    pub fn abort_children(&mut self) {
        for child in self.children.values() {
            child.abort();
        }

        self.children.clear();
    }

    pub fn task<'a>(&'a mut self) -> SpawnBuilder<'a, Args> {
        SpawnBuilder::new(self, ())
    }

    pub fn task_with<'a, ChildArgs>(
        &'a mut self,
        args: ChildArgs,
    ) -> SpawnBuilder<'a, Args, ChildArgs>
    where
        ChildArgs: Send + 'static,
    {
        SpawnBuilder::new(self, args)
    }
}

pub struct SpawnBuilder<'a, ParentArgs, ChildArgs = ()> {
    parent_ctx: &'a mut Ctx<ParentArgs>,
    child_args: ChildArgs,
    on_err: OnErr,
}

impl<'a, ParentArgs, ChildArgs> SpawnBuilder<'a, ParentArgs, ChildArgs>
where
    ParentArgs: Send + 'static,
    ChildArgs: Send + 'static,
{
    pub fn new(parent_ctx: &'a mut Ctx<ParentArgs>, child_args: ChildArgs) -> Self {
        Self {
            parent_ctx,
            child_args,
            on_err: OnErr::Restart {
                max: Limit::None,
                backoff: Backoff::None,
            },
        }
    }

    pub fn args<NewChildArgs>(
        self,
        child_args: NewChildArgs,
    ) -> SpawnBuilder<'a, ParentArgs, NewChildArgs> {
        SpawnBuilder {
            parent_ctx: self.parent_ctx,
            child_args,
            on_err: self.on_err,
        }
    }

    pub fn on_err<NewChildArgs>(mut self, on_err: OnErr) -> Self {
        self.on_err = on_err;
        self
    }

    pub fn spawn<ChildErr, TaskFn, Fut>(self, taskfn: TaskFn) -> Result<()>
    where
        ChildErr: Send + 'static,
        TaskFn: Send + 'static + for<'f> Fn(&'f mut Ctx<ChildArgs>) -> Fut,
        Fut: Future<Output = Result<(), ChildErr>> + Send + 'static,
    {
        self.inner_spawn(taskfn, false).map(|_| ())
    }

    pub fn spawnwatch<ChildErr, TaskFn, Fut>(
        self,
        taskfn: TaskFn,
    ) -> Result<Receiver<(TaskId, ChildErr)>>
    where
        ChildErr: Send + 'static,
        TaskFn: Send + 'static + for<'f> Fn(&'f mut Ctx<ChildArgs>) -> Fut,
        Fut: Future<Output = Result<(), ChildErr>> + Send + 'static,
    {
        self.inner_spawn(taskfn, true)
            .map(|receiver| receiver.unwrap())
    }

    fn inner_spawn<ChildErr, TaskFn, Fut>(
        self,
        taskfn: TaskFn,
        watch: bool,
    ) -> Result<Option<Receiver<(TaskId, ChildErr)>>>
    where
        ChildArgs: Send + 'static,
        ChildErr: Send + 'static,
        TaskFn: Send + 'static + Fn(&mut Ctx<ChildArgs>) -> Fut,
        Fut: Future<Output = Result<(), ChildErr>> + Send + 'static,
    {
        let children = &mut self.parent_ctx.children;

        let next_id = children
            .keys()
            .fold(0, |highest_id, curr_id| cmp::max(highest_id, curr_id.0))
            + 1;

        let next_id = TaskId(next_id);

        let (err_tx, err_rx) = if watch {
            let (tx, rx) = flume::unbounded();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let child_ctx = Ctx {
            args: self.child_args,
            pubsub: self.parent_ctx.pubsub.clone(),
            on_err: self.on_err,
            children: Default::default(),
        };

        let mut restart_count = 0_u64;

        let handle = task::spawn(async move {
            let mut child_ctx = child_ctx;
            let mut err_tx = err_tx;
            let task_id = next_id;

            loop {
                if let Err(e) = taskfn(&mut child_ctx).await {
                    if let Some(tx) = &err_tx {
                        if tx.send((task_id, e)).is_err() {
                            err_tx = None;
                        }
                    }

                    match child_ctx.on_err {
                        OnErr::Stop => break,
                        OnErr::Restart { max, backoff } => {
                            if max == restart_count {
                                break;
                            }

                            let wait = match backoff {
                                Backoff::None => Duration::ZERO,
                                Backoff::Static(duration) => duration,
                                Backoff::Incremental { min, max, step } => {
                                    let wait = step.mul_f64((restart_count + 1) as f64);
                                    let wait = cmp::min(max, wait);
                                    cmp::max(min, wait)
                                }
                            };

                            time::sleep(wait).await;
                        }
                    };
                }

                restart_count += 1;
            }
        });

        children.insert(next_id, handle.abort_handle());

        Ok(err_rx)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OnErr {
    Stop,
    Restart { max: Limit, backoff: Backoff },
}
