mod manager;

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::result::Result;

use crate::epoll::{Epoll, EpollEvent, EventSet};

pub use manager::{ControlOps, SubscriberOps};

#[derive(Debug)]
pub enum Error {
    Epoll(io::Error),
    // TODO: should we allow fds to be registered multiple times?
    FdAlreadyRegistered,
    InvalidToken,
}

#[derive(Clone, Copy)]
pub struct Events {
    inner: EpollEvent,
}

impl Events {
    pub fn empty<T: AsRawFd>(source: &T) -> Self {
        Self::empty_raw(source.as_raw_fd())
    }

    pub fn empty_raw(fd: RawFd) -> Self {
        Self::new_raw(fd, EventSet::empty())
    }

    pub fn new<T: AsRawFd>(source: &T, events: EventSet) -> Self {
        Self::new_raw(source.as_raw_fd(), events)
    }

    pub fn new_raw(source: RawFd, events: EventSet) -> Self {
        Self::with_data_raw(source, 0, events)
    }

    pub fn with_data<T: AsRawFd>(source: &T, data: u32, events: EventSet) -> Self {
        Self::with_data_raw(source.as_raw_fd(), data, events)
    }

    pub fn with_data_raw(source: RawFd, data: u32, events: EventSet) -> Self {
        let inner_data = (source as u64) << 32 + data as u64;
        Events {
            inner: EpollEvent::new(events, inner_data),
        }
    }

    pub fn fd(&self) -> RawFd {
        self.inner.data() as RawFd
    }

    pub fn data(&self) -> u32 {
        (self.inner.data() >> 32) as u32
    }

    pub fn event_set(&self) -> EventSet {
        self.inner.event_set()
    }

    pub fn epoll_event(&self) -> EpollEvent {
        self.inner
    }
}

pub struct SubscriberOpsEndpoint<S> {
    blah: S,
}

impl<S: EventSubscriber> SubscriberOpsEndpoint<S> {
    pub fn invoke<F, O, E>(&self, f: Box<F>) -> Result<O, E>
    where
        F: FnOnce(&mut dyn SubscriberOps<Subscriber = S>) -> Result<O, E> + Send,
        O: Send,
        E: From<Error> + Send,
    {
        unimplemented!()
    }
}

pub trait EventSubscriber {
    fn process(&mut self, events: Events, ops: &ControlOps);
    fn init(&self, ops: &ControlOps);
}
