use std::os::unix::io::{AsRawFd, RawFd};

use crate::epoll::{EpollEvent, EventSet};

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
