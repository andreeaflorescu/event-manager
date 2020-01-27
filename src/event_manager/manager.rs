use std::sync::{Arc, Mutex};

use crate::epoll::{ControlOperation, EpollEvent, EventSet};

use super::endpoint::EventManagerChannel;
use super::epoll_context::EpollContext;
use super::subscribers::Subscribers;
use super::{
    ControlOps, Error, EventSubscriber, Events, RemoteEndpoint, Result, SubscriberOps,
    SubscriberToken,
};

/// Allows event subscribers to be registered, connected to the event loop, and later removed.
pub struct EventManager<T> {
    subscribers: Subscribers<T>,
    epoll_context: EpollContext,
    ready_events: Vec<EpollEvent>,
    channel: EventManagerChannel<T>,
}

impl<T: EventSubscriber> SubscriberOps for EventManager<T> {
    type Subscriber = T;

    /// Register a subscriber with the event event_manager and returns the associated token.
    fn add_subscriber(&mut self, subscriber: T) -> SubscriberToken {
        let token = self.subscribers.add(subscriber);
        let index = token.index;
        self.subscribers
            .get_mut(index)
            // The index is valid because we've just added the subscriber.
            .init(&mut self.epoll_context.ops_unchecked(index));
        token
    }

    /// Unregisters and returns the subscriber associated with the provided token.
    fn remove_subscriber(&mut self, token: SubscriberToken) -> Result<T> {
        let subscriber = self.subscribers.remove(token).ok_or(Error::InvalidToken)?;
        self.epoll_context.remove(token.index);
        Ok(subscriber)
    }

    /// Return a mutable reference to the subscriber associated with the provided token.
    fn subscriber_mut(&mut self, token: SubscriberToken) -> Result<&mut T> {
        self.subscribers.find(token).ok_or(Error::InvalidToken)
    }

    /// Returns a `ControlOps` object for the subscriber associated with the provided token.
    fn control_ops(&mut self, token: SubscriberToken) -> Result<ControlOps> {
        // Check if the token is valid.
        if self.subscribers.find(token).is_some() {
            // The index is valid because the result of `find` was not `None`.
            return Ok(self.epoll_context.ops_unchecked(token.index));
        }
        Err(Error::InvalidToken)
    }
}

// TODO: add implementations for other standard wrappers as well.
impl EventSubscriber for Arc<Mutex<dyn EventSubscriber>> {
    fn process(&mut self, events: Events, ops: &mut ControlOps) {
        self.lock().unwrap().process(events, ops);
    }

    fn init(&self, ops: &mut ControlOps) {
        self.lock().unwrap().init(ops);
    }
}

impl<S: EventSubscriber> EventManager<S> {
    /// Create a new `EventManger` object.
    pub fn new() -> Result<Self> {
        let manager = EventManager {
            subscribers: Subscribers::new(),
            epoll_context: EpollContext::new()?,
            ready_events: vec![EpollEvent::default(); 256],
            channel: EventManagerChannel::new()?,
        };

        let fd = manager.channel.fd();
        manager
            .epoll_context
            .epoll
            .ctl(
                ControlOperation::Add,
                fd,
                EpollEvent::new(EventSet::IN, fd as u64),
            )
            .map_err(Error::Epoll)?;

        Ok(manager)
    }

    /// Run the event loop blocking until the first batch of events.
    // Using the current run interface, not sure if the best but that's another discussion.
    pub fn run(&mut self) -> Result<usize> {
        self.run_with_timeout(-1)
    }

    /// Wait for events for a maximum timeout of `miliseconds`. Dispatch the events to the
    /// registered signal handlers.
    pub fn run_with_timeout(&mut self, milliseconds: i32) -> Result<usize> {
        let event_count = self
            .epoll_context
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

    /// Return a `RemoteEndpoint` object, that allows interacting with the `EventManager` from a
    /// different thread. Using `RemoteEndpoint::call_blocking` on the same thread the event loop
    /// runs on leads to a deadlock.
    pub fn remote_endpoint(&self) -> RemoteEndpoint<S> {
        self.channel.remote_endpoint()
    }

    fn dispatch_events(&mut self, event_count: usize) {
        // Use the temporary, pre-allocated buffer to check ready events.
        for ev_index in 0..event_count {
            let event = self.ready_events[ev_index];
            let fd = event.fd();

            if fd == self.channel.fd() {
                if event.event_set() != EventSet::IN {
                    // This situation is virtually impossible to occur. Should we do anything
                    // about it?
                }
                self.handle_endpoint_calls();
                continue;
            }

            let sub_index = if let Some(i) = self.epoll_context.subscriber_index(fd) {
                i
            } else {
                // TODO: Should we log an error in case the subscriber does not exist?
                continue;
            };

            self.subscribers
                .get_mut(sub_index)
                // The index is valid because the previous call to `context.subscriber_index` did
                // not return `None`.
                .process(
                    Events::with_inner(event),
                    &mut self.epoll_context.ops_unchecked(sub_index),
                );
        }
    }

    fn handle_endpoint_calls(&mut self) {
        // Clear the inner event_fd. We don't do anything about an error here at this point.
        let _ = self.channel.event_fd.read();

        // Process messages. We consider only `Empty` errors can appear here; we don't check
        // for `Disconnected` errors because we keep at least one clone of `channel.sender` alive
        // at all times ourselves.
        while let Ok(msg) = self.channel.receiver.try_recv() {
            // We call the inner closure and attempt to send back the result, but can't really do
            // anything in case of error here.
            let _ = msg.sender.send((msg.fnbox)(self));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::io::{AsRawFd, RawFd};
    use std::thread;

    use vmm_sys_util::eventfd::EventFd;

    use crate::epoll::EventSet;

    struct DummySubscriber {
        event_fd_1: EventFd,
        event_fd_2: EventFd,

        // Flags used for checking that the event event_manager called the `process`
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
                    .add(Events::new(&self.event_fd_2, EventSet::OUT))
                    .unwrap();
                self.register_ev2 = false;
            }

            if self.unregister_ev1 {
                event_manager
                    .remove(Events::new_raw(
                        self.event_fd_1.as_raw_fd(),
                        EventSet::empty(),
                    ))
                    .unwrap();
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

    impl EventSubscriber for DummySubscriber {
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

        let mut event_manager = EventManager::<Arc<Mutex<dyn EventSubscriber>>>::new().unwrap();
        let dummy_subscriber = Arc::new(Mutex::new(DummySubscriber::new()));

        event_manager.add_subscriber(dummy_subscriber.clone());

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
        let mut event_manager = EventManager::<Arc<Mutex<dyn EventSubscriber>>>::new().unwrap();
        let dummy_subscriber = Arc::new(Mutex::new(DummySubscriber::new()));

        event_manager.add_subscriber(dummy_subscriber.clone());

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
        let mut event_manager = EventManager::<Arc<Mutex<dyn EventSubscriber>>>::new().unwrap();
        let dummy_subscriber = Arc::new(Mutex::new(DummySubscriber::new()));

        event_manager.add_subscriber(dummy_subscriber.clone());

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

    #[test]
    fn test_endpoint() {
        let mut event_manager = EventManager::<DummySubscriber>::new().unwrap();
        let dummy = DummySubscriber::new();
        let endpoint = event_manager.remote_endpoint();

        let thread_handle = thread::spawn(move || {
            event_manager.run().unwrap();
        });

        dummy.event_fd_1.write(1).unwrap();

        let token = endpoint
            .call_blocking(|sub_ops| -> Result<SubscriberToken> {
                Ok(sub_ops.add_subscriber(dummy))
            })
            .unwrap();

        assert_eq!(token.index, 0);
        assert_eq!(token.id, 0);

        thread_handle.join().unwrap();
    }
}
