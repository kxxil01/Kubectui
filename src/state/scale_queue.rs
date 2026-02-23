//! Scale request queue for managing pending scaling operations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Status of a scale request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScaleStatus {
    /// Request pending (optimistic UI update shown).
    Pending,
    /// Scale operation succeeded.
    Success,
    /// Scale operation failed.
    Error,
}

/// A single scale request in the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleQueueItem {
    /// Unique request ID (typically UUID or timestamp-based).
    pub id: String,
    /// Deployment name.
    pub deployment: String,
    /// Target namespace.
    pub namespace: String,
    /// Target replica count.
    pub target_replicas: i32,
    /// Current status.
    pub status: ScaleStatus,
    /// When the request was created.
    pub created_at: DateTime<Utc>,
    /// Error message if status is Error.
    pub error: Option<String>,
}

impl ScaleQueueItem {
    /// Creates a new pending scale queue item.
    pub fn new(
        id: impl Into<String>,
        deployment: impl Into<String>,
        namespace: impl Into<String>,
        target_replicas: i32,
    ) -> Self {
        Self {
            id: id.into(),
            deployment: deployment.into(),
            namespace: namespace.into(),
            target_replicas,
            status: ScaleStatus::Pending,
            created_at: Utc::now(),
            error: None,
        }
    }

    /// Marks this item as successfully scaled.
    pub fn mark_success(&mut self) {
        self.status = ScaleStatus::Success;
    }

    /// Marks this item as failed with an error message.
    pub fn mark_error(&mut self, error: impl Into<String>) {
        self.status = ScaleStatus::Error;
        self.error = Some(error.into());
    }
}

/// Queue of pending and completed scale requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScaleQueue {
    items: HashMap<String, ScaleQueueItem>,
}

impl ScaleQueue {
    /// Creates a new empty scale queue.
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    /// Adds a scale request to the queue.
    pub fn add(&mut self, item: ScaleQueueItem) {
        self.items.insert(item.id.clone(), item);
    }

    /// Retrieves a scale request by ID.
    pub fn get(&self, id: &str) -> Option<&ScaleQueueItem> {
        self.items.get(id)
    }

    /// Retrieves a mutable reference to a scale request by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut ScaleQueueItem> {
        self.items.get_mut(id)
    }

    /// Removes a scale request from the queue by ID.
    pub fn remove(&mut self, id: &str) -> Option<ScaleQueueItem> {
        self.items.remove(id)
    }

    /// Returns all items in the queue.
    pub fn all(&self) -> Vec<&ScaleQueueItem> {
        self.items.values().collect()
    }

    /// Returns only pending items.
    pub fn pending(&self) -> Vec<&ScaleQueueItem> {
        self.items
            .values()
            .filter(|item| item.status == ScaleStatus::Pending)
            .collect()
    }

    /// Returns only completed items (success or error).
    pub fn completed(&self) -> Vec<&ScaleQueueItem> {
        self.items
            .values()
            .filter(|item| item.status != ScaleStatus::Pending)
            .collect()
    }

    /// Clears the queue entirely.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Returns the number of items in the queue.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_queue_item_creation() {
        let item = ScaleQueueItem::new("req-1", "my-deploy", "default", 3);
        assert_eq!(item.id, "req-1");
        assert_eq!(item.deployment, "my-deploy");
        assert_eq!(item.namespace, "default");
        assert_eq!(item.target_replicas, 3);
        assert_eq!(item.status, ScaleStatus::Pending);
        assert!(item.error.is_none());
    }

    #[test]
    fn test_scale_queue_item_mark_success() {
        let mut item = ScaleQueueItem::new("req-1", "my-deploy", "default", 3);
        item.mark_success();
        assert_eq!(item.status, ScaleStatus::Success);
    }

    #[test]
    fn test_scale_queue_add_and_get() {
        let mut queue = ScaleQueue::new();
        let item = ScaleQueueItem::new("req-1", "my-deploy", "default", 3);
        queue.add(item.clone());

        let retrieved = queue.get("req-1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "req-1");
    }

    #[test]
    fn test_scale_queue_remove() {
        let mut queue = ScaleQueue::new();
        let item = ScaleQueueItem::new("req-1", "my-deploy", "default", 3);
        queue.add(item);

        let removed = queue.remove("req-1");
        assert!(removed.is_some());
        assert!(queue.get("req-1").is_none());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_scale_queue_pending() {
        let mut queue = ScaleQueue::new();
        let mut item1 = ScaleQueueItem::new("req-1", "deploy1", "default", 2);
        let mut item2 = ScaleQueueItem::new("req-2", "deploy2", "default", 3);

        item2.mark_success();
        queue.add(item1);
        queue.add(item2);

        let pending = queue.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "req-1");
    }
}
