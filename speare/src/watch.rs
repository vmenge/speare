use flume::Sender;

pub trait OnErrTerminate<E>: Send + 'static {
    fn on_err_terminate(&self, err: &E);
}

pub struct NoWatch;

impl<E> OnErrTerminate<E> for NoWatch {
    fn on_err_terminate(&self, _err: &E) {}
}

pub struct WatchFn<F, Msg> {
    pub f: F,
    pub parent_msg_tx: Sender<Msg>,
}

impl<F, E, Msg> OnErrTerminate<E> for WatchFn<F, Msg>
where
    F: Fn(&E) -> Msg + Send + 'static,
    Msg: Send + 'static,
{
    fn on_err_terminate(&self, err: &E) {
        let msg = (self.f)(err);
        let _ = self.parent_msg_tx.send(msg);
    }
}
