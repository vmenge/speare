use crate::{Actor, Ctx};
use std::any::{Any, TypeId};
use std::collections::HashMap;

pub(crate) type SendFn = Box<dyn Fn(&dyn Any) -> bool + Send + Sync>;

pub(crate) struct Subscriber {
    pub(crate) id: u64,
    pub(crate) send_fn: SendFn,
}

pub(crate) struct TopicEntry {
    pub(crate) type_id: TypeId,
    pub(crate) subscribers: Vec<Subscriber>,
}

pub(crate) struct PubSub {
    pub(crate) next_sub_id: u64,
    pub(crate) topics: HashMap<String, TopicEntry>,
}

impl PubSub {
    pub(crate) fn new() -> Self {
        Self {
            next_sub_id: 0,
            topics: HashMap::new(),
        }
    }
}

impl Default for PubSub {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum PubSubError {
    TypeMismatch { topic: String },
    PoisonErr,
}

impl std::fmt::Display for PubSubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PubSubError::TypeMismatch { topic } => {
                write!(f, "pub/sub topic type mismatch for topic: {topic}")
            }

            PubSubError::PoisonErr => write!(f, "pub/sub lock poisoned"),
        }
    }
}

impl std::error::Error for PubSubError {}

impl<P: Actor> Ctx<P> {
    /// Subscribes this actor to a topic. Messages published to the topic will be
    /// cloned, converted via `From<T>` into this actor's `Msg` type, and delivered
    /// to this actor's mailbox.
    ///
    /// Returns `PubSubError::TypeMismatch` if the topic was already created with
    /// a different message type.
    pub fn subscribe<T>(&mut self, topic: &str) -> Result<(), PubSubError>
    where
        T: Clone + Send + 'static,
        P::Msg: From<T>,
    {
        let mut bus = self.pubsub.write().map_err(|_| PubSubError::PoisonErr)?;

        let type_id = TypeId::of::<T>();

        if let Some(entry) = bus.topics.get(topic) {
            if entry.type_id != type_id {
                return Err(PubSubError::TypeMismatch {
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

        let msg_tx = self.handle.msg_tx.clone();
        let send_fn = Box::new(move |any: &dyn Any| -> bool {
            if let Some(val) = any.downcast_ref::<T>() {
                msg_tx.send(val.clone().into()).is_ok()
            } else {
                false
            }
        });

        entry.subscribers.push(Subscriber {
            id: sub_id,
            send_fn,
        });
        self.subscription_ids.push((topic.to_string(), sub_id));

        Ok(())
    }

    /// Publishes a message to all subscribers of a topic. The message is cloned
    /// for each subscriber.
    ///
    /// Publishing to a topic with no subscribers is a no-op and returns `Ok(())`.
    /// Returns `PubSubError::TypeMismatch` if the topic exists with a different type.
    pub fn publish<T: Send + 'static>(&self, topic: &str, msg: T) -> Result<(), PubSubError> {
        let bus = self.pubsub.read().map_err(|_| PubSubError::PoisonErr)?;

        let Some(entry) = bus.topics.get(topic) else {
            return Ok(());
        };

        if entry.type_id != TypeId::of::<T>() {
            return Err(PubSubError::TypeMismatch {
                topic: topic.to_string(),
            });
        }

        let msg_any = &msg as &dyn Any;
        for sub in &entry.subscribers {
            (sub.send_fn)(msg_any);
        }

        Ok(())
    }
}
