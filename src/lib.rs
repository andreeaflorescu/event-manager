// Copyright 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
mod endpoint;
mod events;
pub mod manager;
mod subscribers;

use std::io;
use std::result;

pub use endpoint::RemoteEndpoint;
// For the outside world, EpollWrapper is actually just a structure that offers
// operations for updating events.
pub use events::{EventOperations, Events};
pub use manager::EventManager;

/// Error conditions that may appear during `EventManager` related operations.
#[derive(Debug)]
pub enum Error {
    ChannelSend,
    ChannelRecv,
    Epoll(io::Error),
    EventFd(io::Error),
    // TODO: should we allow fds to be registered multiple times?
    FdAlreadyRegistered,
    InvalidId,
    InvalidEvent,
}

/// Generic result type that may return `EventManager` errors.
pub type Result<T> = result::Result<T, Error>;

/// Opaque object that uniquely represents a subscriber registered with an `EventManager`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct SubscriberId(u64);

/// The `EventSubscriber` trait allows the interaction between an `EventManager` and different
/// event subscribers.
pub trait EventSubscriber {
    /// Respond to events and potentially alter the interest set of the subscriber.
    ///
    /// Called by the `EventManager` whenever an event associated with the subscriber is triggered.
    fn process(&mut self, events: Events, ops: &mut EventOperations);

    /// Register the events initially associated with the subscriber.
    ///
    /// Called by the `EventManager` after a subscriber is registered.
    fn init(&self, subcriber_id: SubscriberId, ops: &mut EventOperations);
}

/// Represents the part of the event event_manager API that allows users to add, remove, and
/// otherwise interact with registered subscribers.
pub trait SubscriberOps {
    type Subscriber: EventSubscriber;

    fn add_subscriber(&mut self, subscriber: Self::Subscriber) -> SubscriberId;
    fn remove_subscriber(&mut self, subscriber_id: SubscriberId) -> Result<Self::Subscriber>;
    fn subscriber_mut(&mut self, subscriber_id: SubscriberId) -> Result<&mut Self::Subscriber>;
}
