use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;
use std::rc::Weak;

use crate::codec::Codec;
use crate::handler_id::HandlerId;
use crate::messages::ToWorker;
use crate::native_worker::NativeWorkerExt;
use crate::traits::Worker;
use crate::{Callback, Shared};

pub(crate) type ToWorkerQueue<W> = Vec<ToWorker<W>>;
pub(crate) type CallbackMap<W> = HashMap<HandlerId, Weak<dyn Fn(<W as Worker>::Output)>>;

struct WorkerBridgeInner<W>
where
    W: Worker,
{
    // When worker is loaded, queue becomes None.
    pending_queue: Shared<Option<ToWorkerQueue<W>>>,
    callbacks: Shared<CallbackMap<W>>,
    post_msg: Rc<dyn Fn(ToWorker<W>)>,
}

impl<W> fmt::Debug for WorkerBridgeInner<W>
where
    W: Worker,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkerBridgeInner<_>")
    }
}

impl<W> WorkerBridgeInner<W>
where
    W: Worker,
{
    /// Send a message to the worker, queuing the message if necessary
    fn send_message(&self, msg: ToWorker<W>) {
        let mut pending_queue = self.pending_queue.borrow_mut();

        match pending_queue.as_mut() {
            Some(m) => {
                m.push(msg);
            }
            None => {
                (self.post_msg)(msg);
            }
        }
    }
}

impl<W> Drop for WorkerBridgeInner<W>
where
    W: Worker,
{
    fn drop(&mut self) {
        let destroy = ToWorker::Destroy;
        self.send_message(destroy);
    }
}

/// A connection manager for components interaction with workers.
pub struct WorkerBridge<W>
where
    W: Worker,
{
    inner: Rc<WorkerBridgeInner<W>>,
    id: HandlerId,
    _worker: PhantomData<W>,
    cb: Option<Rc<dyn Fn(W::Output)>>,
}

impl<W> WorkerBridge<W>
where
    W: Worker,
{
    pub(crate) fn new<CODEC>(
        id: HandlerId,
        native_worker: web_sys::Worker,
        pending_queue: Rc<RefCell<Option<ToWorkerQueue<W>>>>,
        callbacks: Rc<RefCell<CallbackMap<W>>>,
        callback: Option<Callback<W::Output>>,
    ) -> Self
    where
        CODEC: Codec,
    {
        let post_msg =
            { move |msg: ToWorker<W>| native_worker.post_packed_message::<_, CODEC>(msg) };

        Self {
            inner: WorkerBridgeInner {
                pending_queue,
                callbacks,
                post_msg: Rc::new(post_msg),
            }
            .into(),
            id,
            _worker: PhantomData,
            cb: callback,
        }
    }

    /// Send a message to the current worker.
    pub fn send(&self, msg: W::Input) {
        let msg = ToWorker::ProcessInput(self.id, msg);
        self.inner.send_message(msg);
    }

    /// Forks the bridge with a different callback.
    ///
    /// This creates a new [HandlerID] that helps the worker to differentiate bridges.
    pub fn fork<F>(&self, cb: Option<F>) -> Self
    where
        F: 'static + Fn(W::Output),
    {
        let cb = cb.map(|m| Rc::new(m) as Rc<dyn Fn(W::Output)>);
        let handler_id = HandlerId::new();

        if let Some(cb_weak) = cb.as_ref().map(Rc::downgrade) {
            self.inner
                .callbacks
                .borrow_mut()
                .insert(handler_id, cb_weak);
        }

        Self {
            inner: self.inner.clone(),
            id: handler_id,
            _worker: PhantomData,
            cb,
        }
    }
}

impl<W> Clone for WorkerBridge<W>
where
    W: Worker,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            id: self.id,
            _worker: PhantomData,
            cb: self.cb.clone(),
        }
    }
}

impl<W> Drop for WorkerBridge<W>
where
    W: Worker,
{
    fn drop(&mut self) {
        let disconnected = ToWorker::Disconnected(self.id);
        self.inner.send_message(disconnected);
    }
}

impl<W> fmt::Debug for WorkerBridge<W>
where
    W: Worker,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WorkerBridge<_>")
    }
}

impl<W> PartialEq for WorkerBridge<W>
where
    W: Worker,
{
    fn eq(&self, rhs: &Self) -> bool {
        self.id == rhs.id
    }
}
