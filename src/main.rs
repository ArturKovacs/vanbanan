use std::{collections::HashSet, io::Cursor, sync::Arc};

use futures::future::join_all;

// use hyper_util::rt::TokioIo;
use log::{debug, error, info};
use tokio::sync::Mutex;

use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::{get, post},
};

use tower_http::services::ServeDir;

use web_push::*;

struct PushSender {
    subscriptions: Mutex<HashSet<SubscriptionInfo>>,
    vapid_private_key: String,
    vapid_public_key: String,
    client: HyperWebPushClient,
}

impl PushSender {
    /// vapid_private_key should be a PEM-encoded EC private key
    fn new(vapid_private_key: String, vapid_public_key: String) -> Self {
        Self {
            subscriptions: Mutex::new(HashSet::with_capacity(50)),
            vapid_private_key,
            vapid_public_key,
            client: HyperWebPushClient::new(),
        }
    }

    async fn add_subscription(&self, subscription_info: SubscriptionInfo) {
        let mut subscriptions = self.subscriptions.lock().await;
        debug!(
            "Adding subscription. Total subscriptions: {}. New subscription: {:?}",
            subscriptions.len(),
            subscription_info
        );
        subscriptions.insert(subscription_info);
    }

    async fn remove_subscription(&self, subscription_info: &SubscriptionInfo) {
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.remove(subscription_info);
    }

    async fn send_push_message(
        &self,
        payload: &[u8],
        ttl: Option<u32>,
    ) -> Result<(), WebPushError> {
        let subscriptions = self.subscriptions.lock().await;
        let futures = subscriptions.iter().map(async |subscription_info| {
            let result = self
                .send_push_message_for_single(subscription_info, payload, ttl)
                .await;
            if let Err(error) = result {
                error!("An error occured: {:?}", error);
            }
        });

        join_all(futures).await;

        Ok(())
    }

    async fn send_push_message_for_single(
        &self,
        subscription_info: &SubscriptionInfo,
        payload: &[u8],
        ttl: Option<u32>,
    ) -> Result<(), WebPushError> {
        let mut builder = WebPushMessageBuilder::new(subscription_info);

        builder.set_payload(ContentEncoding::Aes128Gcm, payload);

        if let Some(seconds) = ttl {
            builder.set_ttl(seconds);
        }

        let cursor = Cursor::new(&self.vapid_private_key);

        let mut sig_builder = VapidSignatureBuilder::from_pem(cursor, subscription_info).unwrap();

        sig_builder.add_claim("sub", "mailto:test@example.com");
        sig_builder.add_claim("foo", "bar");
        sig_builder.add_claim("omg", 123);

        let signature = sig_builder.build().unwrap();
        builder.set_vapid_signature(signature);

        self.client.send(builder.build()?).await
    }
}

async fn subscription_handler(
    State(push_sender): State<Arc<PushSender>>,
    Json(subscription_info): Json<SubscriptionInfo>,
) -> impl IntoResponse {
    push_sender.add_subscription(subscription_info).await;
    "Subscription added"
}

async fn post_message_handler(
    State(push_sender): State<Arc<PushSender>>,
    payload: String,
) -> impl IntoResponse {
    debug!("Distributing push message to subscribers: {}", payload);

    let result = push_sender
        .send_push_message(payload.as_bytes(), Some(60))
        .await;
    match result {
        Ok(_) => info!("Push message distributed successfully"),
        Err(e) => {
            error!("Failed to send push message: {:?}", e);
            return Err("");
        }
    }
    Ok("Message sent")
}

async fn get_public_key_handler(State(push_sender): State<Arc<PushSender>>) -> impl IntoResponse {
    push_sender.vapid_public_key.clone()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    env_logger::init();

    let vapid_private_key = std::fs::read_to_string("vapid_private_key.pem")?;
    let vapid_public_key = std::fs::read_to_string("vapid_public_key.txt")?;

    let shared_state = Arc::new(PushSender::new(vapid_private_key, vapid_public_key));

    let static_dir = ServeDir::new("../liba-front").append_index_html_on_directories(true);

    let app = Router::new()
        .route("/hello", get(async || "Hello, World!"))
        .route("/api/subscription", post(subscription_handler))
        .route("/api/message", post(post_message_handler))
        .route("/api/public-key", get(get_public_key_handler))
        .fallback_service(static_dir)
        .with_state(shared_state);

    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}
