use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::result::Result;

use crate::epoll::{Epoll, EpollEvent, EventSet};

pub enum Error {}

pub struct Events {
    inner: EpollEvent,
}

impl Events {
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
}

#[derive(Clone, Copy)]
pub struct SubscriberToken {
    index: usize,
}

pub struct EventManager<T> {
    epoll: Epoll,
    subscribers: Vec<T>,
    fd_dispatch: HashMap<RawFd, usize>,
}

impl<T: EventSubscriber> EventManager<T> {
    pub fn add_subscriber(&mut self, subscriber: T) -> SubscriberToken {
        self.subscribers.push(subscriber);
        let token = SubscriberToken {
            index: self.subscribers.len() - 1,
        };

        let registered_events = RegisteredEvents::new(&self.epoll, &mut self.fd_dispatch, &token);
        self.subscribers[token.index].init(&registered_events);
        token
    }

    pub fn remove_subscriber(&mut self, token: SubscriberToken) -> Result<T, Error> {
        unimplemented!()
    }

    pub fn subscriber(&self, token: SubscriberToken) -> Result<&T, Error> {
        unimplemented!()
    }

    pub fn subscriber_mut(&mut self, token: SubscriberToken) -> Result<&mut T, Error> {
        unimplemented!()
    }
}

pub struct RegisteredEvents<'a> {
    epoll: &'a Epoll,
    fd_dispatch: &'a mut HashMap<RawFd, usize>,
    token: &'a SubscriberToken,
}

impl<'a> RegisteredEvents<'a> {
    fn new(
        epoll: &'a Epoll,
        fd_dispatch: &'a mut HashMap<RawFd, usize>,
        token: &'a SubscriberToken,
    ) -> Self {
        RegisteredEvents {
            epoll,
            fd_dispatch,
            token,
        }
    }

    pub fn add(&self, events: Events) {}

    pub fn modify(&self, events: Events) {}

    pub fn remove<T: AsRawFd>(&self, events: Events) {}
}

pub struct EventManagerEndpoint<T> {
    blah: T,
}

impl<T: EventSubscriber> EventManagerEndpoint<T> {
    pub fn invoke<F, O, E>(&self, f: Box<F>) -> Result<O, E>
    where
        F: FnOnce(&mut EventManager<T>) -> Result<O, E> + Send,
        O: Send,
        E: From<Error> + Send,
    {
        unimplemented!()
    }
}

pub trait EventSubscriber {
    fn process(&mut self, events: Events, registered_events: &RegisteredEvents);
    fn init(&self, registered_events: &RegisteredEvents);
}
