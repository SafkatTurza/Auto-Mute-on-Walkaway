//! Synchronous in-process event bus.
//!
//! The application is event-driven: the core emits [`DomainEvent`]s and the
//! outer layers (logging, tray, UI bridge, notifications) react to them without
//! knowing about each other. This bus is that seam — a simple synchronous
//! fan-out to registered subscribers.
//!
//! It is intentionally synchronous and lock-guarded rather than async: event
//! volume is tiny (a handful per minute), subscribers are cheap, and avoiding a
//! runtime keeps startup fast and idle CPU near zero — both hard MVP targets.

use std::sync::{Arc, Mutex};

use amow_domain::DomainEvent;

/// A subscriber callback. `Send + Sync` so the bus can live behind an `Arc` and
/// be shared across threads (e.g. the detection thread and the UI thread).
pub type Subscriber = Box<dyn Fn(&DomainEvent) + Send + Sync>;

/// Stored form of a subscriber. Held as `Arc` so `publish` can take a cheap
/// snapshot under the lock and invoke callbacks with the lock released.
type StoredSubscriber = Arc<dyn Fn(&DomainEvent) + Send + Sync>;

/// Fan-out event bus.
///
/// Cloning an [`EventBus`] yields another handle to the *same* bus, so
/// publishers and the subscriber registry can be freely shared.
#[derive(Clone, Default)]
pub struct EventBus {
    subscribers: Arc<Mutex<Vec<StoredSubscriber>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a subscriber. Every subsequently published event is delivered
    /// to it in registration order.
    pub fn subscribe(&self, subscriber: Subscriber) {
        self.subscribers
            .lock()
            .expect("event bus mutex poisoned")
            .push(Arc::from(subscriber));
    }

    /// Publish an event to all current subscribers synchronously.
    ///
    /// A snapshot of subscriber handles is cloned under the lock and the lock
    /// is released before any callback runs, so a subscriber may itself
    /// `subscribe` or `publish` without deadlocking. Subscribers registered
    /// during this call receive subsequent events, not the in-flight one.
    pub fn publish(&self, event: &DomainEvent) {
        let snapshot: Vec<StoredSubscriber> = {
            let guard = self.subscribers.lock().expect("event bus mutex poisoned");
            guard.clone()
        };
        for sub in &snapshot {
            sub(event);
        }
    }

    /// Number of registered subscribers (primarily for tests/diagnostics).
    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .expect("event bus mutex poisoned")
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amow_domain::{MeetingState, PresenceState};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn presence_evt() -> DomainEvent {
        DomainEvent::PresenceChanged {
            state: PresenceState::Away,
            at: 1,
        }
    }

    #[test]
    fn delivers_to_all_subscribers_in_order() {
        let bus = EventBus::new();
        let log = Arc::new(Mutex::new(Vec::<usize>::new()));
        for id in 0..3 {
            let log = log.clone();
            bus.subscribe(Box::new(move |_| log.lock().unwrap().push(id)));
        }
        bus.publish(&presence_evt());
        assert_eq!(*log.lock().unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn counts_events_received() {
        let bus = EventBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        bus.subscribe(Box::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        bus.publish(&presence_evt());
        bus.publish(&DomainEvent::MeetingChanged {
            state: MeetingState::Active,
            at: 2,
        });
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn subscriber_added_during_delivery_is_retained() {
        let bus = EventBus::new();
        let bus2 = bus.clone();
        // A subscriber that registers another subscriber on first event.
        bus.subscribe(Box::new(move |_| {
            if bus2.subscriber_count() == 1 {
                bus2.subscribe(Box::new(|_| {}));
            }
        }));
        bus.publish(&presence_evt());
        assert_eq!(bus.subscriber_count(), 2);
    }
}
