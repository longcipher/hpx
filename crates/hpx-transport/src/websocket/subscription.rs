//! Lock-free subscription management using `scc::HashMap`.
//!
//! This store manages topic subscriptions with reference counting,
//! enabling multiple subscribers per topic and automatic cleanup.

use std::sync::Arc;

use tokio::sync::broadcast;

use super::{config::WsConfig, protocol::WsMessage, types::Topic};

/// Internal subscription entry with sender and reference count.
struct SubscriptionEntry {
    sender: broadcast::Sender<WsMessage>,
    ref_count: usize,
}

/// Lock-free store for WebSocket subscriptions.
///
/// Uses `scc::HashMap` for wait-free reads and lock-free writes.
/// Multiple subscribers can listen to the same topic via broadcast channels.
pub struct SubscriptionStore {
    subscriptions: scc::HashMap<Topic, SubscriptionEntry>,
    config: Arc<WsConfig>,
}

impl SubscriptionStore {
    /// Create a new subscription store.
    pub fn new(config: Arc<WsConfig>) -> Self {
        Self {
            subscriptions: scc::HashMap::new(),
            config,
        }
    }

    /// Subscribe to a topic, returning a receiver for messages.
    ///
    /// If the topic is already subscribed, returns a new receiver for
    /// the existing channel. Otherwise, creates a new channel.
    ///
    /// Returns `(receiver, is_new)` where `is_new` is true if this is
    /// a new subscription that needs to be sent to the server.
    pub fn subscribe(&self, topic: Topic) -> (broadcast::Receiver<WsMessage>, bool) {
        // Try to get existing subscription and increment ref count atomically
        if let Some(receiver) = self.subscriptions.update_sync(&topic, |_, entry| {
            entry.ref_count += 1;
            entry.sender.subscribe()
        }) {
            return (receiver, false);
        }

        // Create new subscription
        let (sender, receiver) = broadcast::channel(self.config.subscription_channel_capacity);
        let entry = SubscriptionEntry {
            sender,
            ref_count: 1,
        };

        // Insert; if another thread beat us, subscribe to their channel
        if let Err((_, _entry)) = self.subscriptions.insert_sync(topic.clone(), entry) {
            // Race: another thread inserted first
            // The entry we tried to insert is returned on error

            if let Some(receiver) = self.subscriptions.update_sync(&topic, |_, entry| {
                entry.ref_count += 1;
                entry.sender.subscribe()
            }) {
                return (receiver, false);
            }
        }

        (receiver, true)
    }

    /// Add a subscriber to an existing topic.
    ///
    /// Returns `Some(receiver)` if the topic exists, `None` otherwise.
    pub fn add_subscriber(&self, topic: &Topic) -> Option<broadcast::Receiver<WsMessage>> {
        self.subscriptions.update_sync(topic, |_, entry| {
            entry.ref_count += 1;
            entry.sender.subscribe()
        })
    }

    /// Unsubscribe from a topic.
    ///
    /// Returns `true` if this was the last subscriber (topic removed),
    /// `false` if there are remaining subscribers.
    pub fn unsubscribe(&self, topic: &Topic) -> bool {
        let mut should_remove = false;

        let _ = self.subscriptions.update_sync(topic, |_, entry| {
            entry.ref_count = entry.ref_count.saturating_sub(1);
            if entry.ref_count == 0 {
                should_remove = true;
            }
        });

        if should_remove {
            self.subscriptions.remove_sync(topic);
            return true;
        }

        false
    }

    /// Publish a message to a topic.
    ///
    /// Returns `true` if the topic exists (even if no active receivers),
    /// `false` if the topic doesn't exist.
    pub fn publish(&self, topic: &Topic, message: WsMessage) -> bool {
        self.subscriptions
            .update_sync(topic, |_, entry| {
                // Ignore send errors (no active receivers is fine)
                let _ = entry.sender.send(message.clone());
            })
            .is_some()
    }

    /// Get all currently subscribed topics.
    pub fn get_all_topics(&self) -> Vec<Topic> {
        let mut topics = Vec::new();
        self.subscriptions.retain_sync(|topic, _| {
            topics.push(topic.clone());
            true
        });
        topics
    }

    /// Get the subscriber count for a topic.
    pub fn subscriber_count(&self, topic: &Topic) -> usize {
        self.subscriptions
            .update_sync(topic, |_, entry| entry.ref_count)
            .unwrap_or(0)
    }

    /// Get the total number of topics.
    pub fn len(&self) -> usize {
        self.subscriptions.len()
    }

    /// Check if there are no subscriptions.
    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }

    /// Clear all subscriptions.
    pub fn clear(&self) {
        self.subscriptions.clear_sync();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Arc<WsConfig> {
        Arc::new(WsConfig::new("wss://test.com"))
    }

    #[test]
    fn test_subscribe_new_topic() {
        let store = SubscriptionStore::new(test_config());
        let topic = Topic::new("orderbook.BTC");

        let (_, is_new) = store.subscribe(topic.clone());
        assert!(is_new);
        assert_eq!(store.len(), 1);
        assert_eq!(store.subscriber_count(&topic), 1);
    }

    #[test]
    fn test_subscribe_existing_topic() {
        let store = SubscriptionStore::new(test_config());
        let topic = Topic::new("orderbook.BTC");

        let (_, is_new1) = store.subscribe(topic.clone());
        assert!(is_new1);

        let (_, is_new2) = store.subscribe(topic.clone());
        assert!(!is_new2);

        assert_eq!(store.len(), 1);
        assert_eq!(store.subscriber_count(&topic), 2);
    }

    #[test]
    fn test_unsubscribe_decrements_count() {
        let store = SubscriptionStore::new(test_config());
        let topic = Topic::new("orderbook.BTC");

        store.subscribe(topic.clone());
        store.subscribe(topic.clone());

        let removed = store.unsubscribe(&topic);
        assert!(!removed);
        assert_eq!(store.subscriber_count(&topic), 1);

        let removed = store.unsubscribe(&topic);
        assert!(removed);
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_publish() {
        let store = SubscriptionStore::new(test_config());
        let topic = Topic::new("trades.ETH");

        let (mut rx, _) = store.subscribe(topic.clone());

        let published = store.publish(&topic, WsMessage::text("test message"));
        assert!(published);

        let received = rx.try_recv();
        assert!(received.is_ok());
    }

    #[test]
    fn test_get_all_topics() {
        let store = SubscriptionStore::new(test_config());

        store.subscribe(Topic::new("topic1"));
        store.subscribe(Topic::new("topic2"));
        store.subscribe(Topic::new("topic3"));

        let topics = store.get_all_topics();
        assert_eq!(topics.len(), 3);
    }
}
