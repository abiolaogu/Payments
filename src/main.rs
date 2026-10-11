//! OpenSASE Payments - Self-hosted Payment Processing

use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sase_payments::security::{bearer, verify_provider_webhook, Introspection};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;
use validator::Validate;

// =============================================================================
// Domain Models
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Transaction {
    pub id: Uuid,
    pub reference: String,
    pub amount: Decimal,
    pub currency: String,
    pub status: String,
    pub transaction_type: String,
    pub customer_id: Option<Uuid>,
    pub customer_email: Option<String>,
    pub payment_method: Option<String>,
    pub provider: Option<String>,
    pub provider_reference: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Wallet {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub balance: Decimal,
    pub currency: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentMethod {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub method_type: String,
    pub provider: String,
    pub token: String,
    pub last_four: Option<String>,
    pub brand: Option<String>,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Refund {
    pub id: Uuid,
    pub transaction_id: Uuid,
    pub amount: Decimal,
    pub reason: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

// =============================================================================
// Application State
// =============================================================================

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub nats: Option<async_nats::Client>,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
}

#[derive(Debug)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub nats_url: Option<String>,
    pub identity_url: String,
    pub allowed_origins: Vec<HeaderValue>,
    pub paystack_secret: Option<String>,
    pub flutterwave_secret: Option<String>,
}

impl Config {
    fn from_env() -> Result<Self> {
        let allowed_origins = std::env::var("ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse::<HeaderValue>()
                    .with_context(|| format!("invalid ALLOWED_ORIGINS entry: {value}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Config {
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8084),
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL required")?,
            nats_url: std::env::var("NATS_URL").ok(),
            identity_url: std::env::var("PLATFORM_IDENTITY_URL")
                .context("PLATFORM_IDENTITY_URL required")?,
            allowed_origins,
            paystack_secret: std::env::var("PAYSTACK_SECRET_KEY").ok(),
            flutterwave_secret: std::env::var("FLUTTERWAVE_SECRET_KEY").ok(),
        })
    }
}

// =============================================================================
// Request/Response DTOs
// =============================================================================

#[derive(Debug, Deserialize, Validate)]
pub struct InitiatePaymentRequest {
    #[validate(range(min = 1))]
    pub amount: i64,
    pub currency: Option<String>,
    #[validate(email)]
    pub email: String,
    pub customer_id: Option<Uuid>,
    pub payment_method: Option<String>,
    pub callback_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct InitiatePaymentResponse {
    pub reference: String,
    pub authorization_url: Option<String>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyPaymentRequest {
    pub reference: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct RefundRequest {
    pub transaction_id: Uuid,
    #[validate(range(min = 1))]
    pub amount: i64,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct WalletTopupRequest {
    pub customer_id: Uuid,
    #[validate(range(min = 1))]
    pub amount: i64,
    pub currency: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct TransferRequest {
    pub from_wallet_id: Uuid,
    pub to_wallet_id: Uuid,
    #[validate(range(min = 1))]
    pub amount: i64,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub status: Option<String>,
    pub from_date: Option<DateTime<Utc>>,
    pub to_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,opensase_payments=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting OpenSASE Payments...");

    let config = Config::from_env()?;
    let config = Arc::new(config);

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    let nats = if let Some(ref url) = config.nats_url {
        async_nats::connect(url).await.ok()
    } else {
        None
    };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    let state = AppState {
        db,
        nats,
        config: config.clone(),
        http,
    };
    let app = build_router(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!("🚀 OpenSASE Payments listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn build_router(state: AppState) -> Router {
    let allowed_origins = state.config.allowed_origins.clone();
    let router = Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api_routes(state.clone()))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    if allowed_origins.is_empty() {
        router
    } else {
        router.layer(
            CorsLayer::new()
                .allow_origin(allowed_origins)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
        )
    }
}

fn api_routes(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/payments/initiate", post(initiate_payment))
        .route("/payments/verify", post(verify_payment))
        .route("/transactions", get(list_transactions))
        .route("/transactions/:id", get(get_transaction))
        .route("/refunds", post(create_refund).get(list_refunds))
        .route("/wallets", post(create_wallet).get(list_wallets))
        .route("/wallets/:id", get(get_wallet))
        .route("/wallets/:id/topup", post(topup_wallet))
        .route("/transfers", post(create_transfer))
        .route_layer(middleware::from_fn_with_state(state, require_legacy_admin));

    Router::new()
        .merge(protected)
        .route("/payments/webhook", post(webhook_handler))
}

async fn require_legacy_admin(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let token = bearer(
        request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    )
    .map(str::to_owned);
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "Bearer workload token required").into_response();
    };

    let url = format!(
        "{}/api/v1/auth/introspect",
        state.config.identity_url.trim_end_matches('/')
    );
    let response = match state
        .http
        .post(url)
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Identity service unavailable",
            )
                .into_response()
        }
    };
    if response.status() != reqwest::StatusCode::OK {
        return (StatusCode::UNAUTHORIZED, "Workload identity rejected").into_response();
    }
    let identity = match response.json::<Introspection>().await {
        Ok(identity) if identity.authorizes_legacy_admin() => identity,
        _ => {
            return (
                StatusCode::FORBIDDEN,
                "Legacy Payments administrator permission required",
            )
                .into_response()
        }
    };

    request.extensions_mut().insert(identity);
    next.run(request).await
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "opensase-payments",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

fn persistence_error(_: sqlx::Error) -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Payments persistence unavailable".to_string(),
    )
}

// =============================================================================
// Payment Handlers
// =============================================================================

async fn initiate_payment(
    State(state): State<AppState>,
    Json(req): Json<InitiatePaymentRequest>,
) -> Result<Json<InitiatePaymentResponse>, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let reference = format!("TXN-{}", Uuid::now_v7());
    let id = Uuid::now_v7();
    let amount = Decimal::new(req.amount, 2);

    sqlx::query(
        r#"INSERT INTO transactions (id, reference, amount, currency, status, transaction_type, customer_email, metadata, created_at, updated_at)
           VALUES ($1, $2, $3, $4, 'pending', 'payment', $5, $6, NOW(), NOW())"#
    )
    .bind(id)
    .bind(&reference)
    .bind(amount)
    .bind(req.currency.as_deref().unwrap_or("NGN"))
    .bind(&req.email)
    .bind(req.metadata.unwrap_or(serde_json::json!({})))
    .execute(&state.db)
    .await
    .map_err(persistence_error)?;

    // In production, integrate with Paystack/Flutterwave here
    let authorization_url = Some(format!("https://checkout.paystack.com/{}", reference));

    Ok(Json(InitiatePaymentResponse {
        reference,
        authorization_url,
        status: "pending".to_string(),
    }))
}

async fn verify_payment(
    State(state): State<AppState>,
    Json(req): Json<VerifyPaymentRequest>,
) -> Result<Json<Transaction>, (StatusCode, String)> {
    let txn = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE reference = $1")
        .bind(&req.reference)
        .fetch_optional(&state.db)
        .await
        .map_err(persistence_error)?
        .ok_or((StatusCode::NOT_FOUND, "Transaction not found".to_string()))?;

    Ok(Json(txn))
}

async fn webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Bytes,
) -> impl IntoResponse {
    let provider = verify_provider_webhook(
        &payload,
        headers
            .get("x-paystack-signature")
            .and_then(|value| value.to_str().ok()),
        headers
            .get("verif-hash")
            .and_then(|value| value.to_str().ok()),
        state.config.paystack_secret.as_deref(),
        state.config.flutterwave_secret.as_deref(),
    );
    let Some(provider) = provider else {
        return StatusCode::UNAUTHORIZED;
    };

    let event_type = serde_json::from_slice::<serde_json::Value>(&payload)
        .ok()
        .and_then(|value| {
            value
                .get("event")
                .and_then(|event| event.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unspecified".to_string());
    tracing::info!(?provider, event_type, "Verified payment webhook received");
    StatusCode::OK
}

async fn list_transactions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<PaginatedResponse<Transaction>>, (StatusCode, String)> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).min(100);
    let offset = ((page - 1) * per_page) as i64;

    let transactions = sqlx::query_as::<_, Transaction>(
        "SELECT * FROM transactions ORDER BY created_at DESC LIMIT $1 OFFSET $2",
    )
    .bind(per_page as i64)
    .bind(offset)
    .fetch_all(&state.db)
    .await
    .map_err(persistence_error)?;

    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
        .fetch_one(&state.db)
        .await
        .map_err(persistence_error)?;

    Ok(Json(PaginatedResponse {
        data: transactions,
        total: total.0,
        page,
        per_page,
    }))
}

async fn get_transaction(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Transaction>, (StatusCode, String)> {
    let txn = sqlx::query_as::<_, Transaction>("SELECT * FROM transactions WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(persistence_error)?
        .ok_or((StatusCode::NOT_FOUND, "Transaction not found".to_string()))?;

    Ok(Json(txn))
}

async fn create_refund(
    State(state): State<AppState>,
    Json(req): Json<RefundRequest>,
) -> Result<(StatusCode, Json<Refund>), (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let id = Uuid::now_v7();
    let amount = Decimal::new(req.amount, 2);

    let refund = sqlx::query_as::<_, Refund>(
        r#"INSERT INTO refunds (id, transaction_id, amount, reason, status, created_at)
           VALUES ($1, $2, $3, $4, 'pending', NOW()) RETURNING *"#,
    )
    .bind(id)
    .bind(req.transaction_id)
    .bind(amount)
    .bind(&req.reason)
    .fetch_one(&state.db)
    .await
    .map_err(persistence_error)?;

    Ok((StatusCode::CREATED, Json(refund)))
}

async fn list_refunds(
    State(state): State<AppState>,
) -> Result<Json<Vec<Refund>>, (StatusCode, String)> {
    let refunds = sqlx::query_as::<_, Refund>("SELECT * FROM refunds ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await
        .map_err(persistence_error)?;

    Ok(Json(refunds))
}

// =============================================================================
// Wallet Handlers
// =============================================================================

async fn create_wallet(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<Wallet>), (StatusCode, String)> {
    let customer_id = req["customer_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or((StatusCode::BAD_REQUEST, "customer_id required".to_string()))?;

    let id = Uuid::now_v7();

    let wallet = sqlx::query_as::<_, Wallet>(
        r#"INSERT INTO wallets (id, customer_id, balance, currency, status, created_at, updated_at)
           VALUES ($1, $2, 0, 'NGN', 'active', NOW(), NOW()) RETURNING *"#,
    )
    .bind(id)
    .bind(customer_id)
    .fetch_one(&state.db)
    .await
    .map_err(persistence_error)?;

    Ok((StatusCode::CREATED, Json(wallet)))
}

async fn list_wallets(
    State(state): State<AppState>,
) -> Result<Json<Vec<Wallet>>, (StatusCode, String)> {
    let wallets = sqlx::query_as::<_, Wallet>("SELECT * FROM wallets ORDER BY created_at DESC")
        .fetch_all(&state.db)
        .await
        .map_err(persistence_error)?;

    Ok(Json(wallets))
}

async fn get_wallet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Wallet>, (StatusCode, String)> {
    let wallet = sqlx::query_as::<_, Wallet>("SELECT * FROM wallets WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(persistence_error)?
        .ok_or((StatusCode::NOT_FOUND, "Wallet not found".to_string()))?;

    Ok(Json(wallet))
}

async fn topup_wallet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<WalletTopupRequest>,
) -> Result<Json<Wallet>, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let amount = Decimal::new(req.amount, 2);

    let wallet = sqlx::query_as::<_, Wallet>(
        "UPDATE wallets SET balance = balance + $1, updated_at = NOW() WHERE id = $2 AND customer_id = $3 RETURNING *"
    )
    .bind(amount)
    .bind(id)
    .bind(req.customer_id)
    .fetch_optional(&state.db)
    .await
    .map_err(persistence_error)?
    .ok_or((StatusCode::NOT_FOUND, "Wallet not found".to_string()))?;

    Ok(Json(wallet))
}

async fn create_transfer(
    State(state): State<AppState>,
    Json(req): Json<TransferRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if req.from_wallet_id == req.to_wallet_id {
        return Err((
            StatusCode::BAD_REQUEST,
            "Source and destination wallets must differ".to_string(),
        ));
    }
    let amount = Decimal::new(req.amount, 2);
    let mut transaction = state.db.begin().await.map_err(persistence_error)?;

    // Debit source wallet
    let debit =
        sqlx::query("UPDATE wallets SET balance = balance - $1 WHERE id = $2 AND balance >= $1")
            .bind(amount)
            .bind(req.from_wallet_id)
            .execute(&mut *transaction)
            .await
            .map_err(persistence_error)?;
    if debit.rows_affected() != 1 {
        return Err((
            StatusCode::CONFLICT,
            "Source wallet not found or has insufficient balance".to_string(),
        ));
    }

    // Credit destination wallet
    let credit = sqlx::query("UPDATE wallets SET balance = balance + $1 WHERE id = $2")
        .bind(amount)
        .bind(req.to_wallet_id)
        .execute(&mut *transaction)
        .await
        .map_err(persistence_error)?;
    if credit.rows_affected() != 1 {
        return Err((
            StatusCode::NOT_FOUND,
            "Destination wallet not found".to_string(),
        ));
    }
    transaction.commit().await.map_err(persistence_error)?;

    Ok(Json(serde_json::json!({
        "status": "completed",
        "amount": req.amount,
        "from": req.from_wallet_id,
        "to": req.to_wallet_id
    })))
}
