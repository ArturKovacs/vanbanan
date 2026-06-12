use std::{io::Cursor, sync::Arc};

use futures::future::join_all;

// use hyper_util::rt::TokioIo;
use log::{debug, error, info};

use axum::{
    Json, Router,
    extract::{Query, State},
    response::IntoResponse,
    routing::{get, post},
};

use tower_http::services::ServeDir;

use web_push::{
    ContentEncoding, HyperWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient, WebPushError, WebPushMessageBuilder
};

use crate::db::SubscriptionId;

mod db;

/// Data Transfer Objects
mod dto {
    use serde::{Deserialize, Serialize};
    use web_push::SubscriptionInfo;

    #[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
    pub struct ExtendedSubscriptionInfo {
        pub subscription_info: SubscriptionInfo,
        pub floor: u32,
    }
    
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
    pub struct PostMessageBody {
        pub floor: u32,
    }

    #[derive(Debug, Serialize)]
    pub struct GetSubscriptionResponse {
        pub floors: Vec<u32>,
    }
}


struct PushSender {
    database: db::Database,
    vapid_private_key: String,
    vapid_public_key: String,
    client: HyperWebPushClient,
}

impl PushSender {
    /// vapid_private_key should be a PEM-encoded EC private key
    fn new(vapid_private_key: String, vapid_public_key: String) -> Self {
        Self {
            database: db::Database::new(),
            vapid_private_key,
            vapid_public_key,
            client: HyperWebPushClient::new(),
        }
    }

    async fn add_subscription(&self, subscription_info: SubscriptionInfo, floor: db::Floor) {
        self.database.add_subscription(subscription_info, floor).await;
    }

    async fn remove_subscription(&self, subscription_id: &SubscriptionId) {
        self.database.remove_subscription(subscription_id).await;
    }

    async fn send_push_message(
        &self,
        payload: &[u8],
        floor: db::Floor,
        ttl: Option<u32>,
    ) -> Result<(), WebPushError> {
        let subscriptions = self.database.get_subscriptions(floor).await;
        let futures = subscriptions.iter().map(async |subscription_info| {
            self.send_push_message_for_single(subscription_info, payload, ttl)
                .await
        });

        join_all(futures).await;

        Ok(())
    }

    async fn send_push_message_for_single(
        &self,
        subscription_info: &SubscriptionInfo,
        payload: &[u8],
        ttl: Option<u32>,
    ) -> Result<(), ()> {
        let mut builder = WebPushMessageBuilder::new(subscription_info);

        builder.set_payload(ContentEncoding::Aes128Gcm, payload);

        if let Some(seconds) = ttl {
            builder.set_ttl(seconds);
        }

        let cursor = Cursor::new(&self.vapid_private_key);

        let mut sig_builder = match VapidSignatureBuilder::from_pem(cursor, subscription_info) {
            Ok(builder) => builder,
            Err(e) => {
                error!("Failed calling VapidSignatureBuilder::from_pem: {:?}", e);
                return Err(());
            }
        };

        sig_builder.add_claim("sub", "mailto:test@example.com");
        sig_builder.add_claim("foo", "bar");
        sig_builder.add_claim("omg", 123);

        let signature = match sig_builder.build() {
            Ok(signature) => signature,
            Err(e) => {
                error!("Failed calling sig_builder.build: {:?}", e);
                return Err(());
            }
        };
        builder.set_vapid_signature(signature);

        let web_push_message = match builder.build() {
            Ok(message) => message,
            Err(e) => {
                error!("Failed calling WebPushMessageBuilder::build: {:?}", e);
                return Err(());
            }
        };

        if let Err(e) = self.client.send(web_push_message).await {
            error!("Failed calling self.client.send: {:?}", e);
            return Err(());
        }

        Ok(())
    }
}

async fn subscription_handler(
    State(push_sender): State<Arc<PushSender>>,
    Json(subscription_info): Json<dto::ExtendedSubscriptionInfo>,
) -> impl IntoResponse {
    push_sender.add_subscription(subscription_info.subscription_info, db::Floor(subscription_info.floor)).await;
    "Subscription added"
}

async fn subscription_get_handler(
    State(push_sender): State<Arc<PushSender>>,
    Query(subscription_id): Query<SubscriptionId>,
) -> Json<dto::GetSubscriptionResponse> {
    // TODO the subscrtiption id may not be URI decoded. (it must be uri encoded on the sender side)
    info!("Received subscription get request for subscription_id: {:?}", subscription_id);

    let subscription_floors = push_sender.database.get_floors_for_subscription(&subscription_id).await;
    let subscription_floors: Vec<u32> = subscription_floors.into_iter().map(|floor| floor.0).collect();
    Json(dto::GetSubscriptionResponse { floors: subscription_floors })
}

async fn post_message_handler(
    State(push_sender): State<Arc<PushSender>>,
    Json(body): Json<dto::PostMessageBody>,
) -> impl IntoResponse {
    debug!("Distributing push message to subscribers for floor: {}", body.floor);

    let payload = serde_json::to_string(&body).map_err(|e| {
        error!("Failed to serialize PostMessageBody: {:?}", e);
        ""
    })?;

    let result = push_sender
        .send_push_message(payload.as_bytes(), db::Floor(body.floor), Some(60))
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

    let static_dir = ServeDir::new("./static")
        .append_index_html_on_directories(true)
        .fallback(get(serve_index));

    let app = Router::new()
        .route("/floor/{floor_id}", get(serve_index))
        .route("/debug", get(serve_index))
        .route("/hello", get(async || "Hello, World!"))
        .route("/api/subscription", post(subscription_handler).get(subscription_get_handler))
        .route("/api/message", post(post_message_handler))
        .route("/api/public-key", get(get_public_key_handler))
        .fallback_service(static_dir)
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

#[axum::debug_handler]
async fn serve_index() -> Result<axum::response::Html<String>, String> {
    let index_content =
        std::fs::read_to_string("./static/index.html").map_err(|e| e.to_string())?;
    Ok(axum::response::Html(index_content))
}
