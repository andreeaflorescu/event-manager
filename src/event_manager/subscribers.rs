use super::SubscriberToken;

use std::collections::HashMap;

// Internal structure used to keep the set of subscribers registered with an EventManger. The
// current implementation uses an index in a HashMap to identify a subscriber, and this implementation
// detail leaks into other internal structures/functionality (EpollContext, ControlOps, etc),
// but is hidden with respect to the public interface of the EventManager.
pub(crate) struct Subscribers<T> {
    // The first element of each pair, when not None, holds a subscriber, while the second
    // records its unique id.
    subscribers: HashMap<u128, T>,
    // We are generating the unique ids by incrementing this counter for each added subscriber,
    // and rely on the extremely large value range of u128 to ensure each value is effectively
    // unique over the runtime of any VMM.
    next_token_id: u128,
}

impl<T> Subscribers<T> {
    pub(crate) fn new() -> Self {
        Subscribers {
            subscribers: HashMap::new(),
            next_token_id: 0,
        }
    }

    // Adds a subscriber and generates an unique token to represent it.
    pub(crate) fn add(&mut self, subscriber: T) -> SubscriberToken {
        let token_id = self.next_token_id;
        self.next_token_id += 1;

        self.subscribers.insert(token_id, subscriber);

        SubscriberToken { id: token_id }
    }

    // Remove and return the subscriber associated with the given token, if it exists.
    pub(crate) fn remove(&mut self, token: SubscriberToken) -> Option<T> {
        self.subscribers.remove(&token.id)
    }

    // Return a mutable reference to the subscriber present at the given index in the inner HashMap.
    // This method should only be called for indices that are known to be valid, otherwise
    // panics can occur.
    pub(crate) fn get_mut(&mut self, token: SubscriberToken) -> Option<&mut T> {
        self.subscribers.get_mut(&token.id)
    }
}
