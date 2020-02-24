use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};

use crate::epoll::{ControlOperation, Epoll, EpollEvent, EventSet};

use super::{Error, EventSubscriber, Events};

#[derive(Clone, Copy)]
pub struct SubscriberToken {
    index: usize,
    id: u128,
}

pub trait SubscriberOps {
    type Subscriber: EventSubscriber;

    fn add_subscriber(&mut self, subscriber: Self::Subscriber) -> SubscriberToken;
    fn remove_subscriber(&mut self, token: SubscriberToken) -> Result<Self::Subscriber, Error>;
    fn subscriber_mut(&mut self, token: SubscriberToken) -> Result<&mut Self::Subscriber, Error>;
    fn control_ops(&mut self, token: SubscriberToken) -> ControlOps;
}

pub struct EventManager<T> {
    epoll: Epoll,
    ready_events: Vec<EpollEvent>,

    next_token_id: u128,
    free_sub_indices: Vec<usize>,

    subscribers: Vec<(Option<T>, u128)>,
    fd_dispatch: HashMap<RawFd, usize>,
    subscriber_watch_list: HashMap<usize, Vec<RawFd>>,
}

impl<T: EventSubscriber> SubscriberOps for EventManager<T> {
    type Subscriber = T;

    fn add_subscriber(&mut self, subscriber: T) -> SubscriberToken {
        let token_id = self.next_token_id;
        self.next_token_id += 1;

        let index = if let Some(idx) = self.free_sub_indices.pop() {
            self.subscribers[idx] = (Some(subscriber), token_id);
            idx
        } else {
            self.subscribers.push((Some(subscriber), token_id));
            self.subscribers.len() - 1
        };

        let token = SubscriberToken {
            index,
            id: token_id,
        };

        // Not using the Self::ops method because we want to borrow individual fields, lest
        // the borrow checker starts complaining on the next line over.
        let ops = ControlOps {
            epoll: &self.epoll,
            fd_dispatch: &mut self.fd_dispatch,
            subscriber_watch_list: &mut self.subscriber_watch_list,
            subscriber_index: token.index,
        };

        // Unwrap will not fail because we've just added the subscriber.
        self.subscribers[token.index].0.as_mut().unwrap().init(&ops);
        token
    }

    fn remove_subscriber(&mut self, token: SubscriberToken) -> Result<T, Error> {
        let (maybe_sub, token_id) = self
            .subscribers
            .get_mut(token.index)
            .ok_or(Error::InvalidToken)?;

        if *token_id != token.id {
            return Err(Error::InvalidToken);
        }

        let sub = maybe_sub.take().ok_or(Error::InvalidToken)?;
        self.free_sub_indices.push(token.index);

        let fds = self.subscriber_watch_list.remove(&token.index).unwrap_or(Vec::new());
        for fd in fds {
            // We ignore the result of the operation since there's nothing we can't do, and its
            // not a significant error condition at this point.
            let _ = self
                .epoll
                .ctl(ControlOperation::Delete, fd, EpollEvent::default());
            self.fd_dispatch.remove(&fd);
        }

        Ok(sub)
    }

    fn subscriber_mut(&mut self, token: SubscriberToken) -> Result<&mut T, Error> {
        let (maybe_sub, token_id) = self
            .subscribers
            .get_mut(token.index)
            .ok_or(Error::InvalidToken)?;

        if *token_id != token.id {
            return Err(Error::InvalidToken);
        }

        maybe_sub.as_mut().ok_or(Error::InvalidToken)
    }

    fn control_ops(&mut self, token: SubscriberToken) -> ControlOps {
        // TODO: verify whether token is valid!
        ControlOps::new(&self.epoll, &mut self.fd_dispatch, &mut self.subscriber_watch_list, token.index)
    }
}

impl<T: EventSubscriber> EventManager<T> {
    // Using the current run interface, not sure if the best but that's another discussion.
    pub fn run(&mut self) -> Result<usize, Error> {
        self.run_with_timeout(-1)
    }

    /// Wait for events for a maximum timeout of `miliseconds`. Dispatch the events to the
    /// registered signal handlers.
    pub fn run_with_timeout(&mut self, milliseconds: i32) -> Result<usize, Error> {
        let event_count = self
            .epoll
            .wait(
                self.ready_events.len(),
                milliseconds,
                &mut self.ready_events[..],
            )
            // .map_err(Error::Poll)?;
            .expect("TODO error handling");
        self.dispatch_events(event_count);

        Ok(event_count)
    }

    fn dispatch_events(&mut self, event_count: usize) {
        // Use the temporary, pre-allocated buffer to check ready events.
        for ev_index in 0..event_count {
            let event = self.ready_events[ev_index];
            let fd = event.fd();

            let sub_index = if let Some(i) = self.fd_dispatch.get(&fd) {
                *i
            } else {
                // TODO: Should we log an error in case the subscriber does not exist?
                continue;
            };

            // Unwrap is ok here because the subscriber has to be present.
            let subscriber = self.subscribers[sub_index].0.as_mut().unwrap();
            let ops = ControlOps {
                epoll: &self.epoll,
                fd_dispatch: &mut self.fd_dispatch,
                subscriber_watch_list: &mut self.subscriber_watch_list,
                subscriber_index: sub_index,
            };
            subscriber.process(Events { inner: event }, &ops);
        }
    }
}

pub struct ControlOps<'a> {
    epoll: &'a Epoll,
    fd_dispatch: &'a mut HashMap<RawFd, usize>,
    subscriber_watch_list: &'a mut HashMap<usize, Vec<RawFd>>,
    subscriber_index: usize,
}

impl<'a> ControlOps<'a> {
    fn new(
        epoll: &'a Epoll,
        fd_dispatch: &'a mut HashMap<RawFd, usize>,
        subscriber_watch_list: &'a mut HashMap<usize, Vec<RawFd>>,
        subscriber_index: usize,
    ) -> Self {
        ControlOps {
            epoll,
            fd_dispatch,
            subscriber_watch_list,
            subscriber_index,
        }
    }

    fn ctl(&self, op: ControlOperation, events: Events) -> Result<(), Error> {
        self.epoll
            .ctl(op, events.fd(), events.epoll_event())
            .map_err(Error::Epoll)
    }

    pub fn add(&mut self, events: Events) -> Result<(), Error> {
        let fd = events.fd();
        if self.fd_dispatch.contains_key(&fd) {
            return Err(Error::FdAlreadyRegistered);
        }
        self.ctl(ControlOperation::Add, events)?;
        self.fd_dispatch.insert(fd, self.subscriber_index);
        self.subscriber_watch_list.get_mut(&self.subscriber_index).unwrap().push(fd);
        Ok(())
    }

    pub fn modify(&self, events: Events) -> Result<(), Error> {
        self.ctl(ControlOperation::Modify, events)
    }

    pub fn remove(&mut self, events: Events) -> Result<(), Error> {
        // TODO: Add some more checks here?
        self.ctl(ControlOperation::Delete, events)?;
        self.fd_dispatch.remove(&events.fd());
        // TODO: don't unwrap here.
        let watch_list = self.subscriber_watch_list.get_mut(&self.subscriber_index).unwrap();
        // TODO: do not unwrap here.
        let index = watch_list.iter().position(|x| *x == events.fd()).unwrap();
        watch_list.remove(index);
        Ok(())
    }
}
