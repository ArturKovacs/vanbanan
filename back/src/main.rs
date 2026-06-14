use std::{io::{self, Cursor}, sync::Arc};

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
    ContentEncoding, HyperWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient, WebPushMessageBuilder,
};

use crate::db::{BananaState, Floor, SubscriptionId};

mod db;

/// Data Transfer Objects
mod dto {
    use std::collections::HashMap;

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

    #[derive(Debug, Deserialize)]
    pub struct SubscriptionDeleteParams {
        pub endpoint: String,
        pub floor: u32,
    }

    #[derive(Debug, Deserialize)]
    pub struct BananaStateForFloor {
        pub floor: u32,
        pub has_banana: bool,
    }

    /// - key: A floor
    /// - value: `true` if there are bananas at the floor given by key. `false` otherwise.
    pub type BananaStates = HashMap<u32, bool>;
}

struct Application {
    database: db::Database,
    vapid_private_key: String,
    vapid_public_key: String,
    client: HyperWebPushClient,
}

impl Application {
    /// vapid_private_key should be a PEM-encoded EC private key
    fn new(vapid_private_key: String, vapid_public_key: String) -> Self {
        Self {
            database: db::Database::new( Default::default()),
            vapid_private_key,
            vapid_public_key,
            client: HyperWebPushClient::new(),
        }
    }

    async fn add_subscription(&self, subscription_info: SubscriptionInfo, floor: db::Floor) -> Result<(), io::Error> {
        self.database
            .add_subscription(subscription_info, floor)
            .await
    }

    async fn remove_subscription(&self, subscription_id: &SubscriptionId, floor: db::Floor) -> Result<(), io::Error> {
        self.database
            .remove_subscription(subscription_id, floor)
            .await
    }

    async fn get_banana_states(&self) -> Result<BananaState, io::Error> {
        self.database.get_banana_states().await
    }

    async fn set_banana_for_floor(&self, floor: db::Floor, has_banana: bool) -> Result<(), io::Error> {
        self.database.set_banana_state_for_floor(floor, has_banana).await
    }

    async fn send_push_message(
        &self,
        payload: &[u8],
        floor: db::Floor,
        ttl: Option<u32>,
    ) -> Result<(), String> {
        let subscriptions = self.database.get_subscriptions(floor).await.map_err(|e| format!("Error during get_subscriptions. {e}"))?;
        let futures = subscriptions.iter().map(async |subscription_info| {
            self.send_push_message_for_single(subscription_info, payload, ttl)
                .await
        });

        let results = join_all(futures).await;
        for result in results {
            result.map_err(|_| format!("Failed to send_push_message_for_single."))?;
        }

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

async fn handle_posting_subscription(
    State(application): State<Arc<Application>>,
    Json(subscription_info): Json<dto::ExtendedSubscriptionInfo>,
) -> Result<(), ()> {
    application
        .add_subscription(
            subscription_info.subscription_info,
            db::Floor(subscription_info.floor),
        )
        .await
        .map_err(|e| {
            error!("Failed to add spubscription. {e}");
        })?;
    Ok(())
}

async fn handle_getting_subscription(
    State(application): State<Arc<Application>>,
    Query(subscription_id): Query<SubscriptionId>,
) -> Result<Json<dto::GetSubscriptionResponse>, ()> {
    // TODO the subscrtiption id may not be URI decoded. (it must be uri encoded on the sender side)
    info!(
        "Received subscription get request for subscription_id: {:?}",
        subscription_id
    );

    let subscription_floors_result = application
        .database
        .get_floors_for_subscription(&subscription_id)
        .await;

    let subscription_floors = match subscription_floors_result {
        Ok(floors) => floors,
        Err(e) => {
            error!("Error occured while get_floors_for_subscription {e}");
            return Err(());
        }
    };

    let subscription_floors: Vec<u32> = subscription_floors
        .into_iter()
        .map(|floor| floor.0)
        .collect();

    Ok(Json(dto::GetSubscriptionResponse {
        floors: subscription_floors,
    }))
}

async fn handle_deleting_subscription(
    State(application): State<Arc<Application>>,
    Query(subscription): Query<dto::SubscriptionDeleteParams>,
) -> Result<(), ()> {
    debug!("Received subscription delete: {:?}", subscription);
    let subscription_id = SubscriptionId {
        endpoint: subscription.endpoint,
    };
    application
        .remove_subscription(&subscription_id, db::Floor(subscription.floor))
        .await
        .map_err(|e| {
            error!("Error during remove_subscription. {e}")
        })?;
    Ok(())
}

async fn handle_posting_message(
    State(application): State<Arc<Application>>,
    Json(body): Json<dto::PostMessageBody>,
) -> Result<(), ()> {
    debug!(
        "Distributing push message to subscribers for floor: {}",
        body.floor
    );

    let payload = serde_json::to_string(&body).map_err(|e| {
        error!("Failed to serialize PostMessageBody: {:?}", e);
    })?;

    let result = application
        .send_push_message(payload.as_bytes(), db::Floor(body.floor), Some(60))
        .await;

    match result {
        Ok(_) => info!("Push message distributed successfully"),
        Err(e) => {
            error!("Failed to send push message: {:?}", e);
            return Err(());
        }
    }
    Ok(())
}

async fn handle_getting_public_key(
    State(application): State<Arc<Application>>,
) -> impl IntoResponse {
    application.vapid_public_key.clone()
}

async fn handle_getting_banana(
    State(application): State<Arc<Application>>,
) -> Result<Json<dto::BananaStates>, ()> {
    application.get_banana_states().await.map_err(|e| {
        error!("Failed to get_banana_states: {e}");
    }).map(|state| {
        Json(state.into_iter().map(|(floor, value)| (floor.0, value)).collect())
    })
}

async fn handle_posting_banana(
    State(application): State<Arc<Application>>,
    Json(banana_state_for_floor): Json<dto::BananaStateForFloor>,
) -> Result<(), ()> {
    let dto::BananaStateForFloor {floor, has_banana} = banana_state_for_floor;
    application.set_banana_for_floor(Floor(floor), has_banana).await.map_err(|e| {
        error!("Failed to set_banana_for_floor: {e}");
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    env_logger::init();

    let vapid_private_key = std::fs::read_to_string("vapid_private_key.pem")?;
    let vapid_public_key = std::fs::read_to_string("vapid_public_key.txt")?;

    let shared_state = Arc::new(Application::new(vapid_private_key, vapid_public_key));

    let static_dir = ServeDir::new("./static")
        .append_index_html_on_directories(true)
        .fallback(get(handle_getting_index));

    let app = Router::new()
        .route("/floor/{floor_id}", get(handle_getting_index))
        .route("/debug", get(handle_getting_index))
        .route("/hello", get(async || "Hello, World!"))
        .route(
            "/api/subscription",
            post(handle_posting_subscription)
                .get(handle_getting_subscription)
                .delete(handle_deleting_subscription),
        )
        .route("/api/message", post(handle_posting_message))
        .route("/api/public-key", get(handle_getting_public_key))
        .route("/api/banana", get(handle_getting_banana).post(handle_posting_banana))
        .fallback_service(static_dir)
        .with_state(shared_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}

#[axum::debug_handler]
async fn handle_getting_index() -> Result<axum::response::Html<String>, String> {
    let index_content =
        std::fs::read_to_string("./static/index.html").map_err(|e| e.to_string())?;
    Ok(axum::response::Html(index_content))
}
