use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::{Arc, Mutex};

use crate::epoll::{ControlOperation, Epoll, EpollEvent, EventSet};

use super::{Error, EventSubscriber, Events};

#[derive(Clone, Copy, Debug)]
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
        let mut ops = ControlOps {
            epoll: &self.epoll,
            fd_dispatch: &mut self.fd_dispatch,
            subscriber_watch_list: &mut self.subscriber_watch_list,
            subscriber_index: token.index,
        };

        // Unwrap will not fail because we've just added the subscriber.
        self.subscribers[token.index].0.as_mut().unwrap().init(&mut ops);
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

impl EventSubscriber for Arc<Mutex<dyn EventSubscriber>> {
    fn process(&mut self, events: Events, ops: &mut ControlOps) {
        self.lock().unwrap().process(events, ops);
    }

    fn init(&self, ops: &mut ControlOps) {
        self.lock().unwrap().init(ops);
    }
}

impl<T: EventSubscriber> EventManager<T> {

    pub fn new() -> Self {
        EventManager {
            epoll: Epoll::new().unwrap(),
            ready_events: vec![EpollEvent::default(); 256],

            next_token_id: 0,
            free_sub_indices: vec![],

            subscribers: vec![],
            fd_dispatch: HashMap::new(),
            subscriber_watch_list: HashMap::new(),
        }
    }

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
            let mut ops = ControlOps {
                epoll: &self.epoll,
                fd_dispatch: &mut self.fd_dispatch,
                subscriber_watch_list: &mut self.subscriber_watch_list,
                subscriber_index: sub_index,
            };
            subscriber.process(Events { inner: event }, &mut ops);
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
        if self.subscriber_watch_list.contains_key(&self.subscriber_index) {
            self.subscriber_watch_list.get_mut(&self.subscriber_index).unwrap().push(fd);
        } else {
            self.subscriber_watch_list.insert(self.subscriber_index, vec![fd]);
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use vmm_sys_util::eventfd::EventFd;

    struct DummySubscriber {
        event_fd_1: EventFd,
        event_fd_2: EventFd,

        // Flags used for checking that the event manager called the `process`
        // function for ev1/ev2.
        processed_ev1_out: bool,
        processed_ev2_out: bool,
        processed_ev1_in: bool,

        // Flags used for driving register/unregister/modify of events from
        // outside of the `process` function.
        register_ev2: bool,
        unregister_ev1: bool,
        modify_ev1: bool,
    }

    impl DummySubscriber {
        fn new() -> Self {
            DummySubscriber {
                event_fd_1: EventFd::new(0).unwrap(),
                event_fd_2: EventFd::new(0).unwrap(),
                processed_ev1_out: false,
                processed_ev2_out: false,
                processed_ev1_in: false,
                register_ev2: false,
                unregister_ev1: false,
                modify_ev1: false,
            }
        }
    }

    impl DummySubscriber {
        fn register_ev2(&mut self) {
            self.register_ev2 = true;
        }

        fn unregister_ev1(&mut self) {
            self.unregister_ev1 = true;
        }

        fn modify_ev1(&mut self) {
            self.modify_ev1 = true;
        }

        fn processed_ev1_out(&self) -> bool {
            self.processed_ev1_out
        }

        fn processed_ev2_out(&self) -> bool {
            self.processed_ev2_out
        }

        fn processed_ev1_in(&self) -> bool {
            self.processed_ev1_in
        }

        fn reset_state(&mut self) {
            self.processed_ev1_out = false;
            self.processed_ev2_out = false;
            self.processed_ev1_in = false;
        }

        fn handle_updates(&mut self, event_manager: &mut ControlOps) {
            if self.register_ev2 {
                event_manager
                    .add(
                        Events::new(&self.event_fd_2, EventSet::OUT),
                    )
                    .unwrap();
                self.register_ev2 = false;
            }

            if self.unregister_ev1 {
                event_manager.remove(Events::new_raw(self.event_fd_1.as_raw_fd(), EventSet::empty())).unwrap();
                self.unregister_ev1 = false;
            }

            if self.modify_ev1 {
                event_manager
                    .modify(Events::new(&self.event_fd_1, EventSet::IN))
                    .unwrap();
                self.modify_ev1 = false;
            }
        }

        fn handle_in(&mut self, source: RawFd) {
            if self.event_fd_1.as_raw_fd() == source {
                self.processed_ev1_in = true;
            }
        }

        fn handle_out(&mut self, source: RawFd) {
            match source {
                _ if self.event_fd_1.as_raw_fd() == source => {
                    self.processed_ev1_out = true;
                }
                _ if self.event_fd_2.as_raw_fd() == source => {
                    self.processed_ev2_out = true;
                }
                _ => {}
            }
        }
    }

    impl EventSubscriber for DummySubscriber{
        fn process(&mut self, events: Events, ops: &mut ControlOps) {
            let source = events.fd();
            let event_set = events.event_set();

            // We only know how to treat EPOLLOUT and EPOLLIN.
            // If we received anything else just stop processing the event.
            let all_but_in_out = EventSet::all() - EventSet::OUT - EventSet::IN;
            if event_set.intersects(all_but_in_out) {
                return;
            }

            self.handle_updates(ops);

            match event_set {
                EventSet::IN => self.handle_in(source),
                EventSet::OUT => self.handle_out(source),
                _ => {}
            }
        }

        fn init(&self, ops: &mut ControlOps) {
            let event = Events::new(&self.event_fd_1, EventSet::OUT);
            ops.add(event).unwrap();
        }
    }

    #[test]
    fn test_register() {
        use super::SubscriberOps;

        let mut event_manager = EventManager::<Arc<Mutex<dyn EventSubscriber>>>::new();
        let dummy_subscriber = Arc::new(Mutex::new(DummySubscriber::new()));

        event_manager
            .add_subscriber(dummy_subscriber.clone());

        dummy_subscriber.lock().unwrap().register_ev2();

        // When running the loop the first time, ev1 should be processed, but ev2 shouldn't
        // because it was just added as part of processing ev1.
        event_manager.run().unwrap();
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev1_out(), true);
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev2_out(), false);

        // Check that both ev1 and ev2 are processed.
        dummy_subscriber.lock().unwrap().reset_state();
        event_manager.run().unwrap();
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev1_out(), true);
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev2_out(), true);
    }

    // Test that unregistering an event while processing another one works.
    #[test]
    fn test_unregister() {
        let mut event_manager = EventManager::<Arc<Mutex<dyn EventSubscriber>>>::new();
        let dummy_subscriber = Arc::new(Mutex::new(DummySubscriber::new()));

        event_manager
            .add_subscriber(dummy_subscriber.clone());

        // Disable ev1. We should only receive this event once.
        dummy_subscriber.lock().unwrap().unregister_ev1();

        event_manager.run().unwrap();
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev1_out(), true);

        dummy_subscriber.lock().unwrap().reset_state();

        // We expect no events to be available. Let's run with timeout so that run exists.
        event_manager.run_with_timeout(100).unwrap();
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev1_out(), false);
    }

    #[test]
    fn test_modify() {
        let mut event_manager = EventManager::<Arc<Mutex<dyn EventSubscriber>>>::new();
        let dummy_subscriber = Arc::new(Mutex::new(DummySubscriber::new()));

        event_manager
            .add_subscriber(dummy_subscriber.clone());

        // Modify ev1 so that it waits for EPOLL_IN.
        dummy_subscriber.lock().unwrap().modify_ev1();
        event_manager.run().unwrap();
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev1_out(), true);
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev2_out(), false);

        dummy_subscriber.lock().unwrap().reset_state();

        // Make sure ev1 is ready for IN so that we don't loop forever.
        dummy_subscriber
            .lock()
            .unwrap()
            .event_fd_1
            .write(1)
            .unwrap();

        event_manager.run().unwrap();
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev1_out(), false);
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev2_out(), false);
        assert_eq!(dummy_subscriber.lock().unwrap().processed_ev1_in(), true);
    }
}
