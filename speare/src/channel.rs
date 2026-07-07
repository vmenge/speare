use crate::{
    error::{Error, Result},
    resource::Res,
    system::{SystemContext, SystemParam},
};
use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
    error, fmt,
    sync::Arc,
};
use tokio::sync::broadcast;

/// Sends messages to a registered channel.
pub struct Sender<T> {
    tx: flume::Sender<T>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<T> Sender<T>
where
    T: Send + Sync + 'static,
{
    /// Sends a message into the channel.
    ///
    /// For bounded channels, this waits until there is capacity. For unbounded
    /// channels, this completes immediately unless all receivers have been
    /// dropped.
    ///
    /// Returns `SendError::Closed` with the original message if all receivers
    /// have been dropped.
    pub async fn send(&self, message: T) -> std::result::Result<(), SendError<T>> {
        self.tx.send_async(message).await.map_err(Into::into)
    }

    /// Attempts to send a message into the channel without waiting.
    ///
    /// Returns `TrySendError::Full` with the original message if this is a
    /// bounded channel and it is currently at capacity.
    ///
    /// Returns `TrySendError::Closed` with the original message if all receivers
    /// have been dropped.
    pub fn try_send(&self, message: T) -> std::result::Result<(), TrySendError<T>> {
        self.tx.try_send(message).map_err(Into::into)
    }
}

/// Receives messages from a registered channel.
///
/// Cloned receivers are competing consumers. Each message is received by one
/// receiver, not broadcast to every receiver.
pub struct Receiver<T>(flume::Receiver<T>);

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Receiver<T>
where
    T: Send + Sync + 'static,
{
    /// Receives the next message from the channel.
    ///
    /// If the channel is empty, this waits until a message is sent or all
    /// senders have been dropped.
    ///
    /// Returns `RecvError::Closed` when all senders have been dropped and no
    /// messages remain in the channel.
    pub async fn recv(&self) -> std::result::Result<T, RecvError> {
        self.0.recv_async().await.map_err(Into::into)
    }

    /// Attempts to receive a message without waiting.
    ///
    /// Returns `TryRecvError::Empty` if no message is currently available.
    ///
    /// Returns `TryRecvError::Closed` when all senders have been dropped and no
    /// messages remain in the channel.
    pub fn try_recv(&self) -> std::result::Result<T, TryRecvError> {
        self.0.try_recv().map_err(Into::into)
    }
}

struct Channel<T> {
    tx: flume::Sender<T>,
    rx: flume::Receiver<T>,
}

#[derive(Clone, Default)]
pub(crate) struct Channels(Arc<HashMap<TypeId, Box<dyn Any + Send + Sync>>>);

impl Channels {
    pub(crate) fn register<T>(
        &mut self,
        channel: (flume::Sender<T>, flume::Receiver<T>),
    ) where
        T: Send + Sync + 'static,
    {
        let channels = Arc::get_mut(&mut self.0).expect("channels are already shared");
        if channels
            .insert(
                TypeId::of::<T>(),
                Box::new(Channel {
                    tx: channel.0,
                    rx: channel.1,
                }),
            )
            .is_some()
        {
            panic!(
                "{} is already registered as a channel. Cannot register the same channel type twice.",
                type_name::<T>()
            );
        }
    }

    pub(crate) fn sender<T>(&self) -> Result<Sender<T>>
    where
        T: Send + Sync + 'static,
    {
        Ok(Sender {
            tx: self.channel::<T>().map_err(Error::source)?.tx.clone(),
        })
    }

    pub(crate) fn receiver<T>(&self) -> Result<Receiver<T>>
    where
        T: Send + Sync + 'static,
    {
        Ok(Receiver(
            self.channel::<T>().map_err(Error::source)?.rx.clone(),
        ))
    }

    fn channel<T>(&self) -> std::result::Result<&Channel<T>, ChannelError>
    where
        T: Send + Sync + 'static,
    {
        self.0
            .get(&TypeId::of::<T>())
            .ok_or(ChannelError::Missing {
                type_name: type_name::<T>(),
            })?
            .downcast_ref::<Channel<T>>()
            .ok_or(ChannelError::TypeMismatch {
                type_name: type_name::<T>(),
            })
    }
}

impl<T> SystemParam for Sender<T>
where
    T: Send + Sync + 'static,
{
    fn extract(context: &SystemContext) -> Result<Self> {
        Res::<Channels>::extract(context)?.sender::<T>()
    }
}

impl<T> SystemParam for Receiver<T>
where
    T: Send + Sync + 'static,
{
    fn extract(context: &SystemContext) -> Result<Self> {
        Res::<Channels>::extract(context)?.receiver::<T>()
    }
}

/// Publishes messages to a registered broadcast.
///
/// Each active `Sub<T>` receives messages published after it subscribes.
pub struct Pub<T> {
    tx: broadcast::Sender<T>,
}

impl<T> Clone for Pub<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<T> Pub<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Publishes a message to all active subscribers.
    ///
    /// This does not wait for subscribers to receive the message. Each active
    /// `Sub<T>` gets its own copy. Subscribers created after this call will not
    /// receive this message.
    ///
    /// Broadcasts are bounded. If a subscriber falls behind by more than the
    /// configured capacity, that subscriber will observe a lag error when it
    /// next receives.
    ///
    /// Returns the number of subscribers that will receive the message.
    ///
    /// Returns `PublishError::Closed` with the original message if there are no
    /// active subscribers.
    pub fn send(&self, message: T) -> std::result::Result<usize, PublishError<T>> {
        self.tx.send(message).map_err(Into::into)
    }
}

/// Receives messages from a registered broadcast.
///
/// Each subscriber receives its own copy of each message published after it
/// subscribes. Slow subscribers may lag and miss messages.
pub struct Sub<T> {
    rx: broadcast::Receiver<T>,
}

impl<T> Sub<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Receives the next broadcast message.
    ///
    /// If no message is available, this waits until a message is published or
    /// the broadcast is closed.
    ///
    /// Returns `BroadcastRecvError::Lagged` if this subscriber fell behind and
    /// older messages were dropped from its buffer.
    ///
    /// Returns `BroadcastRecvError::Closed` if all publishers have been dropped.
    pub async fn recv(&mut self) -> std::result::Result<T, BroadcastRecvError> {
        self.rx.recv().await.map_err(Into::into)
    }

    /// Attempts to receive a broadcast message without waiting.
    ///
    /// Returns `BroadcastTryRecvError::Empty` if no message is currently
    /// available.
    ///
    /// Returns `BroadcastTryRecvError::Lagged` if this subscriber fell behind
    /// and older messages were dropped from its buffer.
    ///
    /// Returns `BroadcastTryRecvError::Closed` if all publishers have been
    /// dropped.
    pub fn try_recv(&mut self) -> std::result::Result<T, BroadcastTryRecvError> {
        self.rx.try_recv().map_err(Into::into)
    }
}

struct Broadcast<T> {
    tx: broadcast::Sender<T>,
}

#[derive(Clone, Default)]
pub(crate) struct Broadcasts(Arc<HashMap<TypeId, Box<dyn Any + Send + Sync>>>);

impl Broadcasts {
    pub(crate) fn register<T>(&mut self, capacity: usize)
    where
        T: Clone + Send + Sync + 'static,
    {
        let broadcasts = Arc::get_mut(&mut self.0).expect("broadcasts are already shared");
        if broadcasts
            .insert(
                TypeId::of::<T>(),
                Box::new(Broadcast::<T> {
                    tx: broadcast::channel(capacity).0,
                }),
            )
            .is_some()
        {
            panic!(
                "{} is already registered as a broadcast. Cannot register the same broadcast type twice.",
                type_name::<T>()
            );
        }
    }

    pub(crate) fn publisher<T>(&self) -> Result<Pub<T>>
    where
        T: Clone + Send + Sync + 'static,
    {
        Ok(Pub {
            tx: self.broadcast::<T>().map_err(Error::source)?.tx.clone(),
        })
    }

    pub(crate) fn subscriber<T>(&self) -> Result<Sub<T>>
    where
        T: Clone + Send + Sync + 'static,
    {
        Ok(Sub {
            rx: self.broadcast::<T>().map_err(Error::source)?.tx.subscribe(),
        })
    }

    fn broadcast<T>(&self) -> std::result::Result<&Broadcast<T>, BroadcastError>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.0
            .get(&TypeId::of::<T>())
            .ok_or(BroadcastError::Missing {
                type_name: type_name::<T>(),
            })?
            .downcast_ref::<Broadcast<T>>()
            .ok_or(BroadcastError::TypeMismatch {
                type_name: type_name::<T>(),
            })
    }
}

impl<T> SystemParam for Pub<T>
where
    T: Clone + Send + Sync + 'static,
{
    fn extract(context: &SystemContext) -> Result<Self> {
        Res::<Broadcasts>::extract(context)?.publisher::<T>()
    }
}

impl<T> SystemParam for Sub<T>
where
    T: Clone + Send + Sync + 'static,
{
    fn extract(context: &SystemContext) -> Result<Self> {
        Res::<Broadcasts>::extract(context)?.subscriber::<T>()
    }
}

#[derive(Debug)]
pub enum ChannelError {
    Missing { type_name: &'static str },
    TypeMismatch { type_name: &'static str },
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { type_name } => {
                write!(f, "{type_name} is not registered as a channel")
            }
            Self::TypeMismatch { type_name } => {
                write!(f, "{type_name} channel had an unexpected type")
            }
        }
    }
}

impl error::Error for ChannelError {}

#[derive(Debug)]
pub enum BroadcastError {
    Missing { type_name: &'static str },
    TypeMismatch { type_name: &'static str },
}

impl fmt::Display for BroadcastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { type_name } => {
                write!(f, "{type_name} is not registered as a broadcast")
            }
            Self::TypeMismatch { type_name } => {
                write!(f, "{type_name} broadcast had an unexpected type")
            }
        }
    }
}

impl error::Error for BroadcastError {}

pub enum SendError<T> {
    Closed(T),
}

impl<T> From<flume::SendError<T>> for SendError<T> {
    fn from(error: flume::SendError<T>) -> Self {
        Self::Closed(error.0)
    }
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(_) => f.debug_tuple("Closed").field(&"<message>").finish(),
        }
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(_) => write!(f, "channel is closed"),
        }
    }
}

impl<T> error::Error for SendError<T> where T: Send + Sync + 'static {}

pub enum TrySendError<T> {
    Full(T),
    Closed(T),
}

impl<T> From<flume::TrySendError<T>> for TrySendError<T> {
    fn from(error: flume::TrySendError<T>) -> Self {
        match error {
            flume::TrySendError::Full(message) => Self::Full(message),
            flume::TrySendError::Disconnected(message) => Self::Closed(message),
        }
    }
}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => f.debug_tuple("Full").field(&"<message>").finish(),
            Self::Closed(_) => f.debug_tuple("Closed").field(&"<message>").finish(),
        }
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => write!(f, "channel is full"),
            Self::Closed(_) => write!(f, "channel is closed"),
        }
    }
}

impl<T> error::Error for TrySendError<T> where T: Send + Sync + 'static {}

#[derive(Debug)]
pub enum RecvError {
    Closed,
}

impl From<flume::RecvError> for RecvError {
    fn from(_: flume::RecvError) -> Self {
        Self::Closed
    }
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "channel is closed"),
        }
    }
}

impl error::Error for RecvError {}

#[derive(Debug)]
pub enum TryRecvError {
    Empty,
    Closed,
}

impl From<flume::TryRecvError> for TryRecvError {
    fn from(error: flume::TryRecvError) -> Self {
        match error {
            flume::TryRecvError::Empty => Self::Empty,
            flume::TryRecvError::Disconnected => Self::Closed,
        }
    }
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "channel is empty"),
            Self::Closed => write!(f, "channel is closed"),
        }
    }
}

impl error::Error for TryRecvError {}

pub enum PublishError<T> {
    Closed(T),
}

impl<T> From<broadcast::error::SendError<T>> for PublishError<T> {
    fn from(error: broadcast::error::SendError<T>) -> Self {
        Self::Closed(error.0)
    }
}

impl<T> fmt::Debug for PublishError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(_) => f.debug_tuple("Closed").field(&"<message>").finish(),
        }
    }
}

impl<T> fmt::Display for PublishError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(_) => write!(f, "broadcast is closed"),
        }
    }
}

impl<T> error::Error for PublishError<T> where T: Send + Sync + 'static {}

#[derive(Debug)]
pub enum BroadcastRecvError {
    Closed,
    Lagged(u64),
}

impl From<broadcast::error::RecvError> for BroadcastRecvError {
    fn from(error: broadcast::error::RecvError) -> Self {
        match error {
            broadcast::error::RecvError::Closed => Self::Closed,
            broadcast::error::RecvError::Lagged(count) => Self::Lagged(count),
        }
    }
}

impl fmt::Display for BroadcastRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "broadcast is closed"),
            Self::Lagged(count) => write!(f, "broadcast receiver lagged by {count} messages"),
        }
    }
}

impl error::Error for BroadcastRecvError {}

#[derive(Debug)]
pub enum BroadcastTryRecvError {
    Empty,
    Closed,
    Lagged(u64),
}

impl From<broadcast::error::TryRecvError> for BroadcastTryRecvError {
    fn from(error: broadcast::error::TryRecvError) -> Self {
        match error {
            broadcast::error::TryRecvError::Empty => Self::Empty,
            broadcast::error::TryRecvError::Closed => Self::Closed,
            broadcast::error::TryRecvError::Lagged(count) => Self::Lagged(count),
        }
    }
}

impl fmt::Display for BroadcastTryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "broadcast receiver is empty"),
            Self::Closed => write!(f, "broadcast is closed"),
            Self::Lagged(count) => write!(f, "broadcast receiver lagged by {count} messages"),
        }
    }
}

impl error::Error for BroadcastTryRecvError {}
