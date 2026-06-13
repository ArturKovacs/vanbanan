
use log::debug;
use serde::Deserialize;
use web_push::{SubscriptionInfo, SubscriptionKeys};
use tokio::sync::Mutex;
use std::collections::{HashMap, HashSet};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Floor(pub u32);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
pub struct SubscriptionId {
    pub endpoint: String,
}

pub struct SubscriptionRest {
    pub keys: SubscriptionKeys,
    pub floors: HashSet<Floor>,
}

pub struct Database {
    subscriptions: Mutex<HashMap<SubscriptionId, SubscriptionRest>>,
}

// TODO: Implement database persistence (e.g., using SQLite or PostgreSQL)

impl Database {
    pub fn new() -> Self {
        Self {
            subscriptions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn add_subscription(&self, subscription_info: SubscriptionInfo, floor: Floor) {
        let mut subscriptions = self.subscriptions.lock().await;
        debug!(
            "Adding subscription. Total subscriptions: {}. New subscription: {:?}",
            subscriptions.len(),
            subscription_info
        );
        let subscription_id = SubscriptionId {
            endpoint: subscription_info.endpoint,
        };
        subscriptions
            .entry(subscription_id)
            .or_insert_with(|| SubscriptionRest {
                keys: subscription_info.keys,
                floors: HashSet::new(),
            })
            .floors
            .insert(floor);
    }

    pub async fn remove_subscription(&self, subscription: &SubscriptionId, floor: Floor) {
        let mut subscriptions = self.subscriptions.lock().await;
        if let Some(subscription) = subscriptions.get_mut(subscription) {
            subscription.floors.remove(&floor);
        }
    }

    /// Returns subscriptions that are subscribed to the given floor
    pub async fn get_subscriptions(&self, floor: Floor) -> HashSet<SubscriptionInfo> {
        let subscriptions = self.subscriptions.lock().await;
        subscriptions
            .iter()
            .filter_map(|(subscription_id, subscription_rest)| {
                if subscription_rest.floors.contains(&floor) {
                    Some(SubscriptionInfo {
                        endpoint: subscription_id.endpoint.clone(),
                        keys: subscription_rest.keys.clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub async fn get_floors_for_subscription(&self, subscription: &SubscriptionId) -> HashSet<Floor> {
        let subscriptions = self.subscriptions.lock().await;
        subscriptions
            .get(subscription)
            .map(|subscription_rest| subscription_rest.floors.clone())
            .unwrap_or_default()
    }
}
