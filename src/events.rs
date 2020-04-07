// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, RawFd};

use super::{Error, Result, SubscriberId};
use vmm_sys_util::epoll::{ControlOperation, Epoll, EpollEvent, EventSet};

/// Wrapper over an `epoll::EpollEvent` object.
///
/// When working directly with epoll related methods, the user associates an `u64` wide
/// epoll_data_t object with every event. We want to use fds as identifiers, but at the same time
/// keep the ability to associate opaque data with an event. An `Events` object always contains an
/// fd and an `u32` data member that can be supplied by the user. When registering events with the
/// inner epoll event set, the fd and data members of `Events` as used together to generate the
/// underlying `u64` member of the epoll_data union.
#[derive(Clone, Copy, Debug)]
pub struct Events {
    inner: EpollEvent,
}

impl PartialEq for Events {
    fn eq(&self, other: &Events) -> bool {
        self.fd() == other.fd()
            && self.data() == other.data()
            && self.event_set() == other.event_set()
    }
}

impl Events {
    pub(crate) fn with_inner(inner: EpollEvent) -> Self {
        Self { inner }
    }

    /// Create an empty event set associated with the supplied fd.
    pub fn empty<T: AsRawFd>(source: &T) -> Self {
        Self::empty_raw(source.as_raw_fd())
    }

    /// Create an empty event set associated with the supplied `RawFd` value.
    pub fn empty_raw(fd: RawFd) -> Self {
        Self::new_raw(fd, EventSet::empty())
    }

    /// Create an event set associated with the supplied fd and active events.
    pub fn new<T: AsRawFd>(source: &T, events: EventSet) -> Self {
        Self::new_raw(source.as_raw_fd(), events)
    }

    /// Create an event set associated with the supplied `RawFd` value and active events.
    pub fn new_raw(source: RawFd, events: EventSet) -> Self {
        Self::with_data_raw(source, 0, events)
    }

    /// Create an event set associated with the supplied fd, active events, and data.
    pub fn with_data<T: AsRawFd>(source: &T, data: u32, events: EventSet) -> Self {
        Self::with_data_raw(source.as_raw_fd(), data, events)
    }

    /// Create an event set associated with the supplied `RawFd` value, active events, and data.
    pub fn with_data_raw(source: RawFd, data: u32, events: EventSet) -> Self {
        let inner_data = ((data as u64) << 32) + (source as u64);
        Events {
            inner: EpollEvent::new(events, inner_data),
        }
    }

    /// Return the inner fd value.
    pub fn fd(&self) -> RawFd {
        self.inner.data() as RawFd
    }

    /// Return the inner data value.
    pub fn data(&self) -> u32 {
        (self.inner.data() >> 32) as u32
    }

    /// Return the active event set.
    pub fn event_set(&self) -> EventSet {
        self.inner.event_set()
    }

    /// Return the inner `EpollEvent`.
    pub fn epoll_event(&self) -> EpollEvent {
        self.inner
    }
}

/// Wrapper over operations that can be executed on `Events`.
// External uses of this structure provide operations that can be executed on Events.
// Internally, this is also used as an `Epoll` wrapper.
pub struct EventOperations {
    // The epoll wrapper.
    pub(crate) epoll: Epoll,
    // Records the id of the subscriber associated with the given RawFd. The event event_manager
    // does not currently support more than one subscriber being associated with an fd.
    fd_dispatch: HashMap<RawFd, SubscriberId>,
    // Records the set of fds that are associated with the subscriber that has the given id.
    // This is used to keep track of all fds associated with a subscriber.
    subscriber_watch_list: HashMap<SubscriberId, Vec<RawFd>>,
}

impl EventOperations {
    pub(crate) fn new() -> Result<Self> {
        Ok(EventOperations {
            epoll: Epoll::new().map_err(Error::Epoll)?,
            fd_dispatch: HashMap::new(),
            subscriber_watch_list: HashMap::new(),
        })
    }

    pub fn modify(&self, event: Events, id: SubscriberId) -> Result<()> {
        if !self.subscriber_event_is_valid(event, id) {
            return Err(Error::InvalidEvent);
        }

        self.epoll
            .ctl(ControlOperation::Modify, event.fd(), event.epoll_event())
            .map_err(Error::Epoll)
    }

    // Remove the `event` associated with the `SubscriberId`. If no such event
    // exists, an error is returned.
    pub fn remove(&mut self, event: Events, id: SubscriberId) -> Result<()> {
        if !self.subscriber_event_is_valid(event, id) {
            return Err(Error::InvalidEvent);
        }

        self.epoll
            .ctl(ControlOperation::Delete, event.fd(), event.epoll_event())
            .map_err(Error::Epoll)?;
        self.fd_dispatch.remove(&event.fd());

        // TODO: If the event is valid, shouldn't it always be present in the
        // watch_list? In which case shouldn't we panic if it doesn't exist instead of
        // actually trying to remove it only if it exists?
        if let Some(watch_list) = self.subscriber_watch_list.get_mut(&id) {
            if let Some(index) = watch_list.iter().position(|&x| x == event.fd()) {
                watch_list.remove(index);
            }
        }
        Ok(())
    }

    pub fn add(&mut self, event: Events, id: SubscriberId) -> Result<()> {
        let fd = event.fd();
        if self.fd_dispatch.contains_key(&fd) {
            return Err(Error::FdAlreadyRegistered);
        }

        self.epoll
            .ctl(ControlOperation::Add, event.fd(), event.epoll_event())
            .map_err(Error::Epoll)?;

        self.fd_dispatch.insert(fd, id);

        self.subscriber_watch_list
            .entry(id)
            .or_insert_with(Vec::new)
            .push(fd);

        Ok(())
    }

    // Helper function to check that the pair (event, subscriber_id) is valid.
    fn subscriber_event_is_valid(&self, event: Events, id: SubscriberId) -> bool {
        if !self.fd_dispatch.contains_key(&event.fd()) {
            return false;
        }

        if *self.fd_dispatch.get(&event.fd()).unwrap() != id {
            return false;
        }
        true
    }

    // Remove the fds associated with the provided subscriber id from the epoll set and the
    // other structures. The subscriber id must be valid.
    pub(crate) fn remove_subscriber(&mut self, subscriber_id: SubscriberId) {
        let fds = self
            .subscriber_watch_list
            .remove(&subscriber_id)
            .unwrap_or_else(Vec::new);
        for fd in fds {
            // We ignore the result of the operation since there's nothing we can't do, and its
            // not a significant error condition at this point.
            let _ = self
                .epoll
                .ctl(ControlOperation::Delete, fd, EpollEvent::default());
            self.fd_dispatch.remove(&fd);
        }
    }

    // Gets the id of the subscriber associated with the provided fd (if such an association
    // exists).
    pub(crate) fn subscriber_id(&self, fd: RawFd) -> Option<SubscriberId> {
        self.fd_dispatch.get(&fd).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use vmm_sys_util::eventfd::EventFd;

    #[test]
    fn test_empty_events() {
        let event_fd = EventFd::new(0).unwrap();

        let events_raw = Events::empty_raw(event_fd.as_raw_fd());
        let events = Events::empty(&event_fd);

        assert_eq!(events, events_raw);

        assert_eq!(events.event_set(), EventSet::empty());
        assert_eq!(events.data(), 0);
        assert_eq!(events.fd(), event_fd.as_raw_fd());
    }

    #[test]
    fn test_events_no_data() {
        let event_fd = EventFd::new(0).unwrap();
        let event_set = EventSet::IN;

        let events_raw = Events::new_raw(event_fd.as_raw_fd(), event_set);
        let events = Events::new(&event_fd, event_set);

        assert_eq!(events_raw, events);

        assert_eq!(events.data(), 0);
        assert_eq!(events.fd(), event_fd.as_raw_fd());
        assert_eq!(events.event_set(), event_set);
    }

    #[test]
    fn test_events_data() {
        let event_fd = EventFd::new(0).unwrap();
        let event_set = EventSet::IN;

        let events_raw = Events::with_data_raw(event_fd.as_raw_fd(), 42, event_set);
        let events = Events::with_data(&event_fd, 43, event_set);

        assert_ne!(events_raw, events);

        assert_eq!(events.data(), 43);
        assert_eq!(events_raw.data(), 42);
    }
}
