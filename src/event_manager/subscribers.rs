use super::SubscriberToken;

// Internal structure used to keep the set of subscribers registered with an EventManger. The
// current implementation uses an index in a Vec to identify a subscriber, and this implementation
// detail leaks into other internal structures/functionality (EpollContext, ControlOps, etc),
// but is hidden with respect to the public interface of the EventManager.
pub(crate) struct Subscribers<T> {
    // The first element of each pair, when not None, holds a subscriber, while the second
    // records its unique id.
    subscribers: Vec<(Option<T>, u128)>,
    // We are generating the unique ids by incrementing this counter for each added subscriber,
    // and rely on the extremely large value range of u128 to ensure each value is effectively
    // unique over the runtime of any VMM.
    next_token_id: u128,
    // Whenever a subscriber is removed, we record its index as being free, so it can be reused
    // when adding other subscribers.
    free_sub_indices: Vec<usize>,
}

impl<T> Subscribers<T> {
    pub(crate) fn new() -> Self {
        Subscribers {
            subscribers: Vec::new(),
            next_token_id: 0,
            free_sub_indices: Vec::new(),
        }
    }

    // Adds a subscriber and generates an unique token to represent it.
    pub(crate) fn add(&mut self, subscriber: T) -> SubscriberToken {
        let token_id = self.next_token_id;
        self.next_token_id += 1;

        let index = if let Some(idx) = self.free_sub_indices.pop() {
            self.subscribers[idx] = (Some(subscriber), token_id);
            idx
        } else {
            self.subscribers.push((Some(subscriber), token_id));
            self.subscribers.len() - 1
        };

        SubscriberToken {
            index,
            id: token_id,
        }
    }

    // Remove and return the subscriber associated with the given token, if it exists.
    pub(crate) fn remove(&mut self, token: SubscriberToken) -> Option<T> {
        if self.find(token).is_some() {
            let (ref mut slot, _) = self.subscribers[token.index];
            self.free_sub_indices.push(token.index);
            return slot.take();
        }
        None
    }

    // Return a mutable reference to the subscribers associated with the provided token, if
    // such a subscriber exists.
    pub(crate) fn find(&mut self, token: SubscriberToken) -> Option<&mut T> {
        let (maybe_sub, token_id) = self.subscribers.get_mut(token.index)?;
        if *token_id != token.id {
            return None;
        }
        maybe_sub.as_mut()
    }

    // Return a mutable reference to the subscriber present at the given index in the inner Vec.
    // This method should only be called for indices that are known to be valid, otherwise
    // panics can occur.
    pub(crate) fn get_mut(&mut self, index: usize) -> &mut T {
        // We use unwrap here because we know the index is valid.
        self.subscribers[index].0.as_mut().unwrap()
    }
}
