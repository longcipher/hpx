//! Integration tests for lock-free WebSocket stores.

use std::{sync::Arc, time::Duration};

use hpx_transport::websocket::{
    PendingRequestStore, RequestId, SubscriptionStore, Topic, WsConfig, WsMessage,
};

fn test_config() -> Arc<WsConfig> {
    Arc::new(
        WsConfig::new("wss://test.com")
            .max_pending_requests(100)
            .request_timeout(Duration::from_secs(5)),
    )
}

// ============================================================================
// PendingRequestStore Tests
// ============================================================================

#[test]
fn test_pending_store_concurrent_add_resolve() {
    use std::thread;

    let store = Arc::new(PendingRequestStore::new(test_config()));
    let mut handles = vec![];

    // Spawn threads that add and resolve requests
    for i in 0..10 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for j in 0..10 {
                let id = RequestId::from(format!("thread{i}-req{j}"));
                let rx = store_clone.add(id.clone(), None);
                assert!(rx.is_some());

                // Resolve immediately
                let resolved = store_clone.resolve(&id, Ok("response".to_string()));
                assert!(resolved);
            }
        }));
    }

    for handle in handles {
        handle.join().ok();
    }

    // All should be resolved
    assert_eq!(store.len(), 0);
}

#[test]
fn test_pending_store_duplicate_id_rejected() {
    let store = PendingRequestStore::new(test_config());
    let id = RequestId::from("same-id");

    let rx1 = store.add(id.clone(), None);
    assert!(rx1.is_some());

    // Duplicate should fail
    let rx2 = store.add(id.clone(), None);
    assert!(rx2.is_none());
}

#[test]
fn test_pending_store_cleanup_preserves_fresh() {
    let config = Arc::new(WsConfig::new("wss://test.com").request_timeout(Duration::from_secs(60))); // Long timeout

    let store = PendingRequestStore::new(config);
    let _rx = store.add(RequestId::from("fresh"), None);

    store.cleanup_stale();
    assert_eq!(store.len(), 1); // Still there
}

#[test]
fn test_pending_store_capacity_limit() {
    let config = Arc::new(
        WsConfig::new("wss://test.com")
            .max_pending_requests(3)
            .request_timeout(Duration::from_secs(60)),
    );

    let store = PendingRequestStore::new(config);

    // Add up to capacity
    let _rx1 = store.add(RequestId::from("req1"), None);
    let _rx2 = store.add(RequestId::from("req2"), None);
    let _rx3 = store.add(RequestId::from("req3"), None);

    assert_eq!(store.len(), 3);
    assert!(!store.has_capacity());

    // Should fail due to capacity
    let rx4 = store.add(RequestId::from("req4"), None);
    assert!(rx4.is_none());
}

#[test]
fn test_pending_store_is_empty() {
    let store = PendingRequestStore::new(test_config());

    assert!(store.is_empty());
    assert_eq!(store.len(), 0);

    let _rx = store.add(RequestId::from("req1"), None);
    assert!(!store.is_empty());
    assert_eq!(store.len(), 1);
}

#[test]
fn test_pending_store_resolve_nonexistent() {
    let store = PendingRequestStore::new(test_config());

    // Resolving non-existent ID should return false
    let resolved = store.resolve(&RequestId::from("nonexistent"), Ok("response".to_string()));
    assert!(!resolved);
}

#[test]
fn test_pending_store_clear_with_error() {
    let store = PendingRequestStore::new(test_config());

    let _rx1 = store.add(RequestId::from("req1"), None);
    let _rx2 = store.add(RequestId::from("req2"), None);
    let _rx3 = store.add(RequestId::from("req3"), None);

    assert_eq!(store.len(), 3);

    store.clear_with_error("connection closed");

    assert_eq!(store.len(), 0);
    assert!(store.is_empty());
}

// ============================================================================
// SubscriptionStore Tests
// ============================================================================

#[test]
fn test_subscription_store_concurrent_subscribe() {
    use std::thread;

    let store = Arc::new(SubscriptionStore::new(test_config()));
    let mut handles = vec![];

    // Multiple threads subscribing to same topic
    for i in 0..10 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            let topic = Topic::new("shared-topic");
            let (_, is_new) = store_clone.subscribe(topic);
            (i, is_new)
        }));
    }

    let results: Vec<_> = handles.into_iter().filter_map(|h| h.join().ok()).collect();

    // Exactly one should be new
    let new_count = results.iter().filter(|(_, is_new)| *is_new).count();
    assert_eq!(new_count, 1);

    // All should be subscribers
    let topic = Topic::new("shared-topic");
    assert_eq!(store.subscriber_count(&topic), 10);
}

#[test]
fn test_subscription_store_publish_all_receive() {
    let store = SubscriptionStore::new(test_config());
    let topic = Topic::new("broadcast-topic");

    // Create multiple subscribers
    let (mut rx1, _) = store.subscribe(topic.clone());
    let (mut rx2, _) = store.subscribe(topic.clone());
    let (mut rx3, _) = store.subscribe(topic.clone());

    // Publish a message
    let published = store.publish(&topic, WsMessage::text("hello all"));
    assert!(published);

    // All should receive
    assert!(rx1.try_recv().is_ok());
    assert!(rx2.try_recv().is_ok());
    assert!(rx3.try_recv().is_ok());
}

#[test]
fn test_subscription_store_unsubscribe_cascade() {
    let store = SubscriptionStore::new(test_config());
    let topic = Topic::new("cascade-topic");

    // Subscribe 3 times
    store.subscribe(topic.clone());
    store.subscribe(topic.clone());
    store.subscribe(topic.clone());

    assert_eq!(store.subscriber_count(&topic), 3);

    // Unsubscribe once
    let removed = store.unsubscribe(&topic);
    assert!(!removed);
    assert_eq!(store.subscriber_count(&topic), 2);

    // Unsubscribe second
    let removed = store.unsubscribe(&topic);
    assert!(!removed);
    assert_eq!(store.subscriber_count(&topic), 1);

    // Last unsubscribe removes topic
    let removed = store.unsubscribe(&topic);
    assert!(removed);
    assert_eq!(store.len(), 0);
}

#[test]
fn test_subscription_store_get_all_topics() {
    let store = SubscriptionStore::new(test_config());

    store.subscribe(Topic::new("topic-a"));
    store.subscribe(Topic::new("topic-b"));
    store.subscribe(Topic::new("topic-c"));

    let topics = store.get_all_topics();
    assert_eq!(topics.len(), 3);
}

#[test]
fn test_subscription_store_publish_to_nonexistent() {
    let store = SubscriptionStore::new(test_config());
    let topic = Topic::new("nonexistent-topic");

    // Publishing to non-existent topic should return false
    let published = store.publish(&topic, WsMessage::text("hello"));
    assert!(!published);
}

#[test]
fn test_subscription_store_is_empty() {
    let store = SubscriptionStore::new(test_config());

    assert!(store.is_empty());
    assert_eq!(store.len(), 0);

    store.subscribe(Topic::new("topic-a"));

    assert!(!store.is_empty());
    assert_eq!(store.len(), 1);
}

#[test]
fn test_subscription_store_add_subscriber_existing() {
    let store = SubscriptionStore::new(test_config());
    let topic = Topic::new("existing-topic");

    // Create initial subscription
    let (_, is_new) = store.subscribe(topic.clone());
    assert!(is_new);
    assert_eq!(store.subscriber_count(&topic), 1);

    // Add subscriber to existing topic
    let rx = store.add_subscriber(&topic);
    assert!(rx.is_some());
    assert_eq!(store.subscriber_count(&topic), 2);
}

#[test]
fn test_subscription_store_add_subscriber_nonexistent() {
    let store = SubscriptionStore::new(test_config());
    let topic = Topic::new("nonexistent-topic");

    // Add subscriber to non-existent topic should return None
    let rx = store.add_subscriber(&topic);
    assert!(rx.is_none());
}

#[test]
fn test_subscription_store_unsubscribe_nonexistent() {
    let store = SubscriptionStore::new(test_config());
    let topic = Topic::new("nonexistent-topic");

    // Unsubscribing from non-existent topic should return false
    let removed = store.unsubscribe(&topic);
    assert!(!removed);
}

// ============================================================================
// Stress Tests
// ============================================================================

#[test]
fn test_stress_mixed_operations() {
    use std::thread;

    let store = Arc::new(SubscriptionStore::new(test_config()));
    let mut handles = vec![];

    // Mix of subscribe, unsubscribe, and publish operations
    for i in 0..20 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for j in 0..50 {
                let topic = Topic::new(format!("topic-{}", j % 5));

                match i % 3 {
                    0 => {
                        store_clone.subscribe(topic);
                    }
                    1 => {
                        store_clone.unsubscribe(&topic);
                    }
                    _ => {
                        store_clone.publish(&topic, WsMessage::text("msg"));
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().ok();
    }

    // Should not panic or deadlock
    assert!(store.len() <= 5);
}

#[test]
fn test_stress_pending_store_mixed_operations() {
    use std::thread;

    let store = Arc::new(PendingRequestStore::new(test_config()));
    let mut handles = vec![];

    // Mix of add, resolve, and cleanup operations
    for i in 0..20 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            for j in 0..50 {
                let id = RequestId::from(format!("req-{}-{}", i, j));

                match i % 4 {
                    0 => {
                        store_clone.add(id, None);
                    }
                    1 => {
                        store_clone.resolve(&id, Ok("response".to_string()));
                    }
                    2 => {
                        store_clone.cleanup_stale();
                    }
                    _ => {
                        // Just check capacity
                        store_clone.has_capacity();
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().ok();
    }

    // Should not panic or deadlock - store may have some pending requests
    assert!(store.len() <= 100); // Within capacity
}

#[test]
fn test_concurrent_publish_receive() {
    use std::thread;

    let store = Arc::new(SubscriptionStore::new(test_config()));
    let topic = Topic::new("concurrent-topic");

    // Create subscribers first
    let (mut rx1, _) = store.subscribe(topic.clone());
    let (mut rx2, _) = store.subscribe(topic.clone());

    // Spawn publishers
    let store_clone = Arc::clone(&store);
    let topic_clone = topic.clone();
    let publisher = thread::spawn(move || {
        for i in 0..100 {
            store_clone.publish(&topic_clone, WsMessage::text(format!("msg-{i}")));
        }
    });

    // Wait for publisher
    publisher.join().ok();

    // Receivers should have messages (may not be all due to channel capacity)
    let mut count1 = 0;
    let mut count2 = 0;

    while rx1.try_recv().is_ok() {
        count1 += 1;
    }
    while rx2.try_recv().is_ok() {
        count2 += 1;
    }

    // Both should have received at least some messages
    assert!(count1 > 0);
    assert!(count2 > 0);
    assert_eq!(count1, count2); // Same number of messages
}

// ============================================================================
// WsMessage Tests
// ============================================================================

#[test]
fn test_ws_message_text() {
    let msg = WsMessage::text("hello");
    assert!(msg.is_text());
    assert!(!msg.is_binary());
    assert_eq!(msg.as_text(), Some("hello"));
    assert_eq!(msg.as_bytes(), b"hello");
}

#[test]
fn test_ws_message_binary() {
    let msg = WsMessage::binary(vec![1, 2, 3, 4]);
    assert!(!msg.is_text());
    assert!(msg.is_binary());
    assert_eq!(msg.as_text(), None);
    assert_eq!(msg.as_bytes(), &[1, 2, 3, 4]);
}

// ============================================================================
// RequestId Tests
// ============================================================================

#[test]
fn test_request_id_from_string() {
    let id1 = RequestId::from("test-id");
    let id2 = RequestId::from(String::from("test-id"));

    assert_eq!(id1.as_str(), "test-id");
    assert_eq!(id2.as_str(), "test-id");
    assert_eq!(id1, id2);
}

#[test]
fn test_request_id_display() {
    let id = RequestId::from("display-test");
    assert_eq!(format!("{id}"), "display-test");
}

#[test]
fn test_request_id_new_unique() {
    let id1 = RequestId::new();
    let id2 = RequestId::new();

    // New IDs should be different
    assert_ne!(id1, id2);
}

// ============================================================================
// Topic Tests
// ============================================================================

#[test]
fn test_topic_new() {
    let topic1 = Topic::new("test-topic");
    let topic2 = Topic::new(String::from("test-topic"));

    assert_eq!(topic1, topic2);
}

#[test]
fn test_topic_clone() {
    let topic1 = Topic::new("cloneable");
    let topic2 = topic1.clone();

    assert_eq!(topic1, topic2);
}
