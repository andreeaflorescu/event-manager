use std::collections::HashMap;
use std::os::unix::io::RawFd;

use crate::epoll::{ControlOperation, Epoll, EpollEvent};

use super::subscribers::SubscriberId;
use super::{ControlOps, Error, Result};

// Internal use structure that keeps the epoll related state of an EventManager.
pub(crate) struct EpollContext {
    // The epoll wrapper.
    pub(crate) epoll: Epoll,
    // Records the unique id of the subscriber associated with the given RawFd. The event_manager
    // does not currently support more than one subscriber being associated with an fd.
    pub(crate) fd_dispatch: HashMap<RawFd, SubscriberId>,
    // Records the set of fds that are associated with the subscriber that has the given unique id
    // (this is required to track all fds ).
    pub(crate) subscriber_watch_list: HashMap<SubscriberId, Vec<RawFd>>,
}

impl EpollContext {
    pub(crate) fn new() -> Result<Self> {
        Ok(EpollContext {
            epoll: Epoll::new().map_err(Error::Epoll)?,
            fd_dispatch: HashMap::new(),
            subscriber_watch_list: HashMap::new(),
        })
    }

    // Remove the fds associated with the provided subscriber id from the epoll set and the
    // other structures. The subscriber id must be valid.
    pub(crate) fn remove(&mut self, subscriber_id: SubscriberId) {
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

    // Gets the unique id of the subscriber associated with the provided fd
    // (if such an association exists).
    pub(crate) fn subscriber_id(&self, fd: RawFd) -> Option<SubscriberId> {
        self.fd_dispatch.get(&fd).copied()
    }

    // Creates and returns a ControlOps object for the subscriber associated with the provided
    // unique id. The subscriber id MUST be valid.
    pub(crate) fn ops_unchecked(&mut self, subscriber_id: SubscriberId) -> ControlOps {
        ControlOps::new(self, subscriber_id)
    }
}
