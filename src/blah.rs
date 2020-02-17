use std::os::unix::io::{AsRawFd, RawFd};

use crate::epoll::EventSet;

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
    fn process(&mut self, fd: u32, data: u32, subscriber_events: &mut dyn SubscriberEvents);
    fn init(&self, subscriber_events: &mut dyn SubscriberEvents);
}

pub trait SubscriberOps {
    type Subscriber: EventSubscriber;

    fn add_subscriber(&mut self, subscriber: Self::Subscriber);
    fn remove_subscriber(&mut self, subscriber: &Self::Subscriber);
}

pub trait EventManager<T: EventSubscriber>: SubscriberOps<Subscriber = T> {}
