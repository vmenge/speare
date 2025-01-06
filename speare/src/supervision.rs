use crate::SharedErr;
use std::{any::Any, cmp, collections::HashMap, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug)]
/// Specifies limits for [`Actor`] restarts in [`Supervision`] strategies.
/// ## Examples
///
/// ```
/// use speare::{Supervision, Limit};
/// use std::time::Duration;
///
/// // Using Limit::Amount to specify a fixed number of restarts
/// let supervision = Supervision::one_for_one()
///     .max_restarts(Limit::Amount(3));
///
/// // Using Limit::Within to specify restarts within a time span
/// let supervision = Supervision::one_for_one()
///     .max_restarts(Limit::Within { amount: 3, timespan: Duration::from_secs(60) });
/// ```
pub enum Limit {
    Amount(u32),
    Within { amount: u32, timespan: Duration },
}

pub trait IntoLimit {
    fn into_limit(self) -> Limit;
}

impl IntoLimit for Limit {
    fn into_limit(self) -> Limit {
        self
    }
}

impl IntoLimit for u32 {
    fn into_limit(self) -> Limit {
        Limit::Amount(self)
    }
}

impl IntoLimit for (u32, Duration) {
    fn into_limit(self) -> Limit {
        Limit::Within {
            amount: self.0,
            timespan: self.1,
        }
    }
}

impl IntoLimit for (Duration, u32) {
    fn into_limit(self) -> Limit {
        Limit::Within {
            amount: self.1,
            timespan: self.0,
        }
    }
}

/// Supervision strategies determine how parent actors respond to child-actor failures,
/// allowing for automated recovery and error handling. Strategies can be customized with
/// max restarts, incremental backoff and conditional directives based on error type.
///
/// **The default strategy is to restart actors on error in a *one-for-one* manner without any limits or backoff.**
///
/// ### One-for-One
/// Applies directives only to the failed actor, maintaining
/// the state of others with the same parent.
///
/// ### One-for-All
/// If any child actor fails, all actors with the same parent
/// will have the same directive applied to them.
///
/// ## Examples
///
/// ```
/// use speare::{Supervision, Directive, Limit, Backoff};
/// use std::time::Duration;
///
/// // One-for-one strategy with a limit on restarts
/// let supervision = Supervision::one_for_one()
///     .directive(Directive::Restart)
///     .max_restarts(Limit::Amount(3));
///
/// // One-for-all strategy with incremental backoff
/// let supervision = Supervision::one_for_all()
///     .directive(Directive::Restart)
///     .backoff(Backoff::Incremental {
///         steps: Duration::from_secs(1),
///         max: Duration::from_secs(10)
///     });
/// ```
#[allow(clippy::type_complexity)]
pub struct Supervision {
    pub(crate) strategy: Strategy,
    pub(crate) directive: Directive,
    pub(crate) max_restarts: Option<Limit>,
    pub(crate) backoff: Option<Backoff>,
    pub(crate) deciders: Vec<Box<dyn Send + Sync + Fn(&Box<dyn Any + Send>) -> Option<Directive>>>,
}

impl Supervision {
    /// Initializes a one-for-one supervision strategy.
    ///
    /// Applies directives only to the failed actor, maintaining
    /// the state of others with the same parent. The default directive is
    /// set to `Directive::Restart`, but can be overridden.
    ///
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Directive};
    ///
    /// Supervision::one_for_one()
    ///     .max_restarts(3)
    ///     .directive(Directive::Resume);
    /// ```
    pub fn one_for_one() -> Self {
        Self {
            strategy: Strategy::OneForOne {
                counter: Default::default(),
            },
            directive: Directive::Restart,
            max_restarts: None,
            backoff: None,
            deciders: vec![],
        }
    }

    /// Initializes a one-for-all supervision strategy.
    ///
    /// If any supervised actor fails, all actors with the same parent
    /// actor will have the same directive applied to them.
    /// The default directive is `Directive::Restart`, but can be overridden.
    ///
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Directive};
    ///
    /// Supervision::one_for_all()
    ///     .max_restarts(5)
    ///     .directive(Directive::Stop);
    /// ```
    pub fn one_for_all() -> Self {
        Self {
            strategy: Strategy::OneForAll {
                counter: Default::default(),
            },
            directive: Directive::Restart,
            max_restarts: None,
            backoff: None,
            deciders: vec![],
        }
    }

    /// Sets the directive to be applied when a child fails.
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Directive};
    ///
    /// let ignore_all_errors = Supervision::one_for_all().directive(Directive::Resume);
    /// let stop_all = Supervision::one_for_all().directive(Directive::Stop);
    /// let restart_all = Supervision::one_for_all().directive(Directive::Restart);
    /// ```
    pub fn directive(mut self, directive: Directive) -> Self {
        self.directive = directive;
        self
    }

    /// Specifies how many times a failed actor will be
    /// restarted before giving up. By default there is no limit.
    /// If limit is reached, actor is stopped.
    ///
    /// ## Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use speare::{Supervision, Directive, Limit};
    ///
    /// // Maximum of 3 restarts during the lifetime of this actor.
    /// Supervision::one_for_one().max_restarts(3);
    ///
    /// Supervision::one_for_one().max_restarts(Limit::Amount(3));
    ///
    /// // Maximum of 3 restarts in a 1 seconds timespan.
    /// Supervision::one_for_one()
    ///     .max_restarts(Limit::Within { amount: 3, timespan: Duration::from_secs(1) });
    ///
    /// Supervision::one_for_one()
    ///     .max_restarts((3, Duration::from_secs(1)));
    /// ```
    pub fn max_restarts<T: IntoLimit>(mut self, max_restarts: T) -> Self {
        self.max_restarts = Some(max_restarts.into_limit());
        self
    }

    /// Configures the backoff strategy for restarts.
    ///
    /// ## Examples
    ///
    /// ```
    /// use speare::{Supervision, Backoff};
    /// use std::time::Duration;
    ///
    /// // Using a static backoff duration
    /// Supervision::one_for_one()
    ///     .backoff(Backoff::Static(Duration::from_secs(5)));
    ///
    /// // Using an incremental backoff
    /// Supervision::one_for_one()
    ///     .backoff(Backoff::Incremental {
    ///         steps: Duration::from_secs(1),
    ///         max: Duration::from_secs(10),
    ///     });
    /// ```
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = Some(backoff);
        self
    }

    /// Allows specifying a closure that decides the directive for a failed actor
    /// based on its error type. The closure is applied only if the error type
    /// successfully downcasts at runtime.
    ///
    /// ## Examples
    ///
    /// ```
    /// use speare::{Ctx, Directive, Node, Actor, Supervision};
    ///
    /// struct Parent;
    ///
    /// impl Actor for Parent {
    ///     type Props = ();
    ///     type Msg = ();
    ///     type Err = ();
    ///
    ///     async fn init(ctx: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///         ctx.spawn::<Foo>(());
    ///         ctx.spawn::<Bar>(());
    ///         Ok(Parent)
    ///     }
    ///
    ///     fn supervision(props: &Self::Props) -> Supervision {
    ///         Supervision::one_for_all()
    ///             .when(|e: &FooErr| Directive::Restart)
    ///             .when(|e: &BarErr| Directive::Stop)
    ///     }
    /// }
    ///
    /// struct Foo;
    ///
    /// struct FooErr(String);
    ///
    /// impl Actor for Foo {
    ///     type Props = ();
    ///     type Msg = ();
    ///     type Err = FooErr;
    ///
    ///     async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///         Err(FooErr("oh no".to_string()))
    ///     }
    /// }
    ///
    /// struct Bar;
    ///
    /// struct BarErr(String);
    ///
    /// impl Actor for Bar {
    ///     type Props = ();
    ///     type Msg = ();
    ///     type Err = BarErr;
    ///
    ///     async fn init(_: &mut Ctx<Self>) -> Result<Self, Self::Err> {
    ///         Err(BarErr("oopsie".to_string()))
    ///     }
    /// }
    /// ```
    pub fn when<F, E>(mut self, f: F) -> Self
    where
        E: 'static,
        F: 'static + Send + Sync + Fn(&E) -> Directive,
    {
        let closure = move |any_e: &Box<dyn Any + Send>| {
            let e: &SharedErr<E> = any_e.downcast_ref()?;
            Some(f(e))
        };

        self.deciders.push(Box::new(closure));

        self
    }
}

type ActorId = u64;

#[derive(Clone, Debug)]
pub(crate) struct RestartCount {
    count: u32,
    last_restart: Instant,
}

impl Default for RestartCount {
    fn default() -> Self {
        Self {
            count: Default::default(),
            last_restart: Instant::now(),
        }
    }
}

impl RestartCount {
    /// None means that max numbers of reset have been reached and we should not reset anymore
    pub(crate) fn get_backoff_duration(
        &mut self,
        max: Option<Limit>,
        backoff: Option<Backoff>,
    ) -> Option<Duration> {
        let (should_restart, new_count) = match max {
            None => (true, self.count + 1),

            Some(Limit::Amount(m)) => (self.count < m, self.count + 1),

            Some(Limit::Within { amount, timespan }) => {
                if self.last_restart + timespan > Instant::now() {
                    (self.count < amount, self.count + 1)
                } else {
                    (true, 1)
                }
            }
        };

        if should_restart {
            let dur = match backoff {
                None => Duration::ZERO,
                Some(Backoff::Static(dur)) => dur,
                Some(Backoff::Incremental { steps, max }) => cmp::min(steps * self.count, max),
            };

            self.count = new_count;
            self.last_restart = Instant::now();

            Some(dur)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Strategy {
    OneForOne {
        counter: HashMap<ActorId, RestartCount>,
    },

    OneForAll {
        counter: RestartCount,
    },
}

#[derive(Clone, Copy, Debug)]
/// Action to take after a `Actor` errors.
pub enum Directive {
    /// Resumes the actor(s) as if nothing happened.
    Resume,
    /// Gracefully stops the actor(s). If the actor is currently handling a message, it will finish handling that and then immediately stop.
    Stop,
    /// Restarts the actor(s), calling `Actor::exit()` and subsequently `Actor::init()`.
    Restart,
    /// Escalate the error to the parent `Actor`.
    Escalate,
}

#[derive(Clone, Copy, Debug)]
/// How long to wait before restarting a `Actor` that errored.
pub enum Backoff {
    /// Uses the same backoff duration for every restart.
    Static(Duration),
    /// Uses a backoff that increases with each subsequent restart up to a maximum value.
    Incremental { steps: Duration, max: Duration },
}
