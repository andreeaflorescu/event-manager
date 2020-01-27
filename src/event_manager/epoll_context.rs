use std::collections::HashMap;
use std::os::unix::io::RawFd;

use crate::epoll::{ControlOperation, Epoll, EpollEvent};

use super::{ControlOps, Error, Result};

// Internal use structure that keeps the epoll related state of an EventManager.
pub(crate) struct EpollContext {
    // The epoll wrapper.
    pub(crate) epoll: Epoll,
    // Records the index of the subscriber associated with the given RawFd. The event event_manager
    // does not currently support more than one subscriber being associated with an fd.
    pub(crate) fd_dispatch: HashMap<RawFd, usize>,
    // Records the set of fds that are associated with the subscriber that has the given index
    // (this is essentially the reverse of the previous field).
    pub(crate) subscriber_watch_list: HashMap<usize, Vec<RawFd>>,
}

impl EpollContext {
    pub(crate) fn new() -> Result<Self> {
        Ok(EpollContext {
            epoll: Epoll::new().map_err(Error::Epoll)?,
            fd_dispatch: HashMap::new(),
            subscriber_watch_list: HashMap::new(),
        })
    }

    // Remove the fds associated with the provided subscriber index from the epoll set and the
    // other structures. The subscriber index must be valid.
    pub(crate) fn remove(&mut self, subscriber_index: usize) {
        let fds = self
            .subscriber_watch_list
            .remove(&subscriber_index)
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

    // Gets the index of the subscriber associated with the provided fd (if such an association
    // exists).
    pub(crate) fn subscriber_index(&self, fd: RawFd) -> Option<usize> {
        self.fd_dispatch.get(&fd).copied()
    }

    // Creates and returns a ControlOps object for the subscriber associated with the provided
    // index. The subscriber index must be valid.
    pub(crate) fn ops_unchecked(&mut self, subscriber_index: usize) -> ControlOps {
        ControlOps::new(self, subscriber_index)
    }
}
