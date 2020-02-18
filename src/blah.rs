use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};

use crate::epoll::{Epoll, EventSet};

pub trait SubscriberEvents {
    // We might consider going for interior mutability here (methods require &self instead of
    // &mut self).

    fn add(&mut self, source: &dyn AsRawFd, events: EventSet, data: u32) {
        self.add_raw(source.as_raw_fd(), events, data);
    }
    fn modify(&mut self, source: &dyn AsRawFd, events: EventSet, data: u32) {
        self.modify_raw(source.as_raw_fd(), events, data);
    }
    fn delete(&mut self, source: &dyn AsRawFd) {
        self.delete_raw(source.as_raw_fd());
    }

    fn add_raw(&mut self, source: RawFd, events: EventSet, data: u32);
    fn modify_raw(&mut self, source: RawFd, events: EventSet, data: u32);
    fn delete_raw(&mut self, source: RawFd);
}

pub trait EventSubscriber {
    fn process(
        &mut self,
        fd: RawFd,
        data: u32,
        events: EventSet,
        subscriber_events: &mut dyn SubscriberEvents,
    );
    fn init(&self, subscriber_events: &mut dyn SubscriberEvents);
}

pub trait SubscriberOps {
    type Subscriber: EventSubscriber;

    fn add_subscriber(&mut self, subscriber: Self::Subscriber);
}

pub trait EventManager: SubscriberOps {}

struct DummyManager<T> {
    epoll: Epoll,
    subscribers: Vec<T>,
    fd_dispatch: HashMap<RawFd, usize>,
}

impl<T: EventSubscriber> DummyManager<T> {
    fn dispatch_events(&mut self, fd: RawFd, data: u32, events: EventSet) {
        // unwrapping for now
        let token = *self.fd_dispatch.get(&fd).unwrap();
        let subscriber = &mut self.subscribers[token];
        let mut blah = Blah::new(&mut self.epoll, &mut self.fd_dispatch, token);
        subscriber.process(fd, data, events, &mut blah);
    }
}

impl<T: EventSubscriber> SubscriberOps for DummyManager<T> {
    type Subscriber = T;

    fn add_subscriber(&mut self, subscriber: T) {
        self.subscribers.push(subscriber)
    }
}

struct Blah<'a> {
    epoll: &'a mut Epoll,
    fd_dispatch: &'a mut HashMap<RawFd, usize>,
    token: usize,
}

impl<'a> Blah<'a> {
    fn new(epoll: &'a mut Epoll, fd_dispatch: &'a mut HashMap<RawFd, usize>, token: usize) -> Self {
        Blah {
            epoll,
            fd_dispatch,
            token,
        }
    }
}

impl<'a> SubscriberEvents for Blah<'a> {
    fn add_raw(&mut self, source: RawFd, events: EventSet, data: u32) {
        self.fd_dispatch.insert(source, self.token);
        // also register actual events with epoll
    }
    fn modify_raw(&mut self, source: RawFd, events: EventSet, data: u32) {
        unimplemented!()
    }
    fn delete_raw(&mut self, source: RawFd) {
        unimplemented!()
    }
}
