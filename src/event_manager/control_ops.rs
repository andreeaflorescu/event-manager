use crate::epoll::ControlOperation;

use super::epoll_context::EpollContext;
use super::subscribers::SubscriberId;
use super::{Error, Events, Result};

/// This opaque object is associated with an `EventSubscriber`, and allows the addition,
/// modification, and removal of file descriptor events within the inner epoll set of an
/// `EventManager`.
// Right now this is a concrete object, but going further it can be turned into a trait and
// passed around as a trait object.
pub struct ControlOps<'a> {
    // Mutable reference to the EpollContext of an EventManager.
    epoll_context: &'a mut EpollContext,
    // The unique id of the event subscriber this object stands for.
    subscriber_id: SubscriberId,
}

impl<'a> ControlOps<'a> {
    pub(crate) fn new(epoll_context: &'a mut EpollContext, subscriber_id: SubscriberId) -> Self {
        ControlOps {
            epoll_context,
            subscriber_id,
        }
    }

    // Apply the provided control operation for the given events on the inner epoll wrapper.
    fn ctl(&self, op: ControlOperation, events: Events) -> Result<()> {
        self.epoll_context
            .epoll
            .ctl(op, events.fd(), events.epoll_event())
            .map_err(Error::Epoll)
    }

    /// Add the provided events to the inner epoll event set.
    pub fn add(&mut self, events: Events) -> Result<()> {
        let fd = events.fd();
        if self.epoll_context.fd_dispatch.contains_key(&fd) {
            return Err(Error::FdAlreadyRegistered);
        }

        self.ctl(ControlOperation::Add, events)?;

        self.epoll_context
            .fd_dispatch
            .insert(fd, self.subscriber_id);

        self.epoll_context
            .subscriber_watch_list
            .entry(self.subscriber_id)
            .or_insert_with(Vec::new)
            .push(fd);

        Ok(())
    }

    /// Submit the provided changes to the inner epoll event set.
    pub fn modify(&self, events: Events) -> Result<()> {
        self.ctl(ControlOperation::Modify, events)
    }

    /// Remove the specified events from the inner epoll event set.
    pub fn remove(&mut self, events: Events) -> Result<()> {
        // TODO: Add some more checks here?
        self.ctl(ControlOperation::Delete, events)?;
        self.epoll_context.fd_dispatch.remove(&events.fd());

        if let Some(watch_list) = self
            .epoll_context
            .subscriber_watch_list
            .get_mut(&self.subscriber_id)
        {
            if let Some(index) = watch_list.iter().position(|&x| x == events.fd()) {
                watch_list.remove(index);
            }
        }
        Ok(())
    }
}
