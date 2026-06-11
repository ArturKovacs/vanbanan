
use web_push::SubscriptionInfo;
use tokio::sync::Mutex;
use std::collections::HashSet;

pub struct ExtendedSubscriptionInfo {
    subscription_info: SubscriptionInfo,
    
}

pub struct Database {
    subscriptions: Mutex<HashSet<SubscriptionInfo>>,
}
