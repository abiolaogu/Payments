use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use sase_payments::{BookingPaymentHold, HoldState, LifecycleError, MutationReceipt, CONTRACT_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, FromRow, PgPool, Postgres, Transaction};
use std::{env, sync::Arc, time::Duration};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    identity_url: String,
    http: Client,
}

#[derive(Debug, Deserialize)]
struct Introspection {
    active: bool,
    #[serde(rename = "sub")]
    subject: String,
    tenant: String,
    #[serde(default)]
    exp: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct AuthorizeRequest {
    booking_reference: String,
    amount: Decimal,
    currency: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct RefundRequest {
    amount: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HoldResponse {
    contract_version: String,
    external_reference: String,
    booking_reference: String,
    amount: Decimal,
    currency: String,
    captured_amount: Decimal,
    refunded_amount: Decimal,
    state: HoldState,
    replayed: bool,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

type ApiError = (StatusCode, Json<ErrorBody>);
type ApiResult<T> = std::result::Result<T, ApiError>;

#[derive(Debug, FromRow)]
struct HoldRow {
    id: Uuid,
    tenant_id: String,
    external_reference: String,
    booking_reference: String,
    amount: Decimal,
    currency: String,
    captured_amount: Decimal,
    refunded_amount: Decimal,
    state: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn api_error(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
    (status, Json(ErrorBody { code, message: message.into() }))
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("authorization")?.to_str().ok()?.trim();
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn header_value(headers: &HeaderMap, name: &'static str) -> String {
    headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().trim().to_string()
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> ApiResult<Introspection> {
    let token = bearer(headers).ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "MISSING_WORKLOAD_TOKEN", "Bearer workload token required"))?;
    let correlation = header_value(headers, "x-correlation-id");
    let url = format!("{}/api/v1/auth/introspect", state.identity_url.trim_end_matches('/'));
    let mut request = state.http.post(url).json(&json!({"token": token}));
    if !correlation.is_empty() {
        request = request.header("X-Correlation-ID", correlation);
    }
    let response = request.send().await.map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "IDENTITY_UNAVAILABLE", "Platform identity service unavailable"))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(api_error(StatusCode::SERVICE_UNAVAILABLE, "IDENTITY_UNAVAILABLE", "Platform identity service rejected introspection"));
    }
    let intro: Introspection = response.json().await.map_err(|_| api_error(StatusCode::SERVICE_UNAVAILABLE, "IDENTITY_UNAVAILABLE", "Malformed Platform identity response"))?;
    if !intro.active || intro.subject.trim().is_empty() || intro.tenant.trim().is_empty() || (intro.exp > 0 && intro.exp <= Utc::now().timestamp()) {
        return Err(api_error(StatusCode::UNAUTHORIZED, "INVALID_WORKLOAD_TOKEN", "Inactive or incomplete workload identity"));
    }
    Ok(intro)
}

fn state_to_db(state: HoldState) -> &'static str {
    match state {
        HoldState::Authorized => "AUTHORIZED",
        HoldState::Captured => "CAPTURED",
        HoldState::Released => "RELEASED",
        HoldState::PartiallyRefunded => "PARTIALLY_REFUNDED",
        HoldState::Refunded => "REFUNDED",
        HoldState::Declined => "DECLINED",
        HoldState::Expired => "EXPIRED",
    }
}

fn state_from_db(raw: &str) -> Result<HoldState, LifecycleError> {
    match raw {
        "AUTHORIZED" => Ok(HoldState::Authorized),
        "CAPTURED" => Ok(HoldState::Captured),
        "RELEASED" => Ok(HoldState::Released),
        "PARTIALLY_REFUNDED" => Ok(HoldState::PartiallyRefunded),
        "REFUNDED" => Ok(HoldState::Refunded),
        "DECLINED" => Ok(HoldState::Declined),
        "EXPIRED" => Ok(HoldState::Expired),
        _ => Err(LifecycleError::InvalidState),
    }
}

impl TryFrom<HoldRow> for BookingPaymentHold {
    type Error = LifecycleError;
    fn try_from(row: HoldRow) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            tenant_id: row.tenant_id,
            external_reference: row.external_reference,
            booking_reference: row.booking_reference,
            amount: row.amount,
            currency: row.currency,
            captured_amount: row.captured_amount,
            refunded_amount: row.refunded_amount,
            state: state_from_db(&row.state)?,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn hold_response(hold: &BookingPaymentHold, replayed: bool) -> HoldResponse {
    HoldResponse {
        contract_version: CONTRACT_VERSION.to_string(),
        external_reference: hold.external_reference.clone(),
        booking_reference: hold.booking_reference.clone(),
        amount: hold.amount,
        currency: hold.currency.clone(),
        captured_amount: hold.captured_amount,
        refunded_amount: hold.refunded_amount,
        state: hold.state,
        replayed,
    }
}

fn lifecycle_error(err: LifecycleError) -> ApiError {
    match err {
        LifecycleError::MissingTenant | LifecycleError::MissingBookingReference | LifecycleError::MissingIdempotencyKey | LifecycleError::InvalidAmount | LifecycleError::InvalidCurrency => api_error(StatusCode::BAD_REQUEST, "INVALID_REQUEST", err.to_string()),
        LifecycleError::IdempotencyConflict => api_error(StatusCode::CONFLICT, "IDEMPOTENCY_CONFLICT", err.to_string()),
        LifecycleError::AlreadyCaptured => api_error(StatusCode::CONFLICT, "ALREADY_CAPTURED", err.to_string()),
        LifecycleError::AlreadyReleased => api_error(StatusCode::CONFLICT, "ALREADY_RELEASED", err.to_string()),
        LifecycleError::AlreadyRefunded => api_error(StatusCode::CONFLICT, "ALREADY_REFUNDED", err.to_string()),
        LifecycleError::Declined => api_error(StatusCode::PAYMENT_REQUIRED, "PAYMENT_DECLINED", err.to_string()),
        LifecycleError::Expired => api_error(StatusCode::GONE, "PAYMENT_EXPIRED", err.to_string()),
        LifecycleError::RefundExceedsCaptured => api_error(StatusCode::CONFLICT, "REFUND_EXCEEDS_CAPTURED", err.to_string()),
        LifecycleError::InvalidState => api_error(StatusCode::CONFLICT, "INVALID_PAYMENT_STATE", err.to_string()),
    }
}

fn idempotency_key(headers: &HeaderMap) -> ApiResult<String> {
    let key = header_value(headers, "idempotency-key");
    if key.is_empty() {
        return Err(lifecycle_error(LifecycleError::MissingIdempotencyKey));
    }
    Ok(key)
}

async fn prior_response(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &str,
    key: &str,
    operation: &str,
    fingerprint: &str,
) -> ApiResult<Option<HoldResponse>> {
    let row: Option<(String, String, Value)> = sqlx::query_as(
        "SELECT operation, fingerprint, response FROM booking_payment_idempotency WHERE tenant_id=$1 AND idempotency_key=$2 FOR UPDATE",
    )
    .bind(tenant)
    .bind(key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_error)?;
    if let Some((prior_operation, prior_fingerprint, response)) = row {
        if prior_operation != operation || prior_fingerprint != fingerprint {
            return Err(lifecycle_error(LifecycleError::IdempotencyConflict));
        }
        let mut parsed: HoldResponse = serde_json::from_value(response).map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "CORRUPT_IDEMPOTENCY_RECORD", "Stored idempotency response is invalid"))?;
        parsed.replayed = true;
        return Ok(Some(parsed));
    }
    Ok(None)
}

fn db_error(_: sqlx::Error) -> ApiError {
    api_error(StatusCode::SERVICE_UNAVAILABLE, "PAYMENTS_UNAVAILABLE", "Payments persistence unavailable")
}

async fn store_idempotency(
    tx: &mut Transaction<'_, Postgres>, tenant: &str, key: &str, operation: &str, fingerprint: &str, response: &HoldResponse,
) -> ApiResult<()> {
    sqlx::query("INSERT INTO booking_payment_idempotency (tenant_id,idempotency_key,operation,fingerprint,response) VALUES ($1,$2,$3,$4,$5)")
        .bind(tenant).bind(key).bind(operation).bind(fingerprint).bind(serde_json::to_value(response).unwrap_or(Value::Null))
        .execute(&mut **tx).await.map_err(db_error)?;
    Ok(())
}

async fn emit_event(
    tx: &mut Transaction<'_, Postgres>, hold: &BookingPaymentHold, event_type: &str, headers: &HeaderMap,
) -> ApiResult<()> {
    let payload = json!({
        "contract_version": CONTRACT_VERSION,
        "external_reference": hold.external_reference,
        "booking_reference": hold.booking_reference,
        "state": hold.state,
        "amount": hold.amount,
        "currency": hold.currency,
        "captured_amount": hold.captured_amount,
        "refunded_amount": hold.refunded_amount,
        "occurred_at": hold.updated_at,
    });
    sqlx::query("INSERT INTO booking_payment_event_outbox (id,tenant_id,aggregate_reference,event_type,payload,correlation_id,causation_id) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(Uuid::now_v7()).bind(&hold.tenant_id).bind(&hold.external_reference).bind(event_type).bind(payload)
        .bind(header_value(headers, "x-correlation-id")).bind(header_value(headers, "x-causation-id"))
        .execute(&mut **tx).await.map_err(db_error)?;
    Ok(())
}

async fn load_hold_for_update(tx: &mut Transaction<'_, Postgres>, tenant: &str, reference: &str) -> ApiResult<BookingPaymentHold> {
    let row = sqlx::query_as::<_, HoldRow>("SELECT * FROM booking_payment_holds WHERE tenant_id=$1 AND external_reference=$2 FOR UPDATE")
        .bind(tenant).bind(reference).fetch_optional(&mut **tx).await.map_err(db_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "PAYMENT_NOT_FOUND", "Payment reference not found"))?;
    row.try_into().map_err(lifecycle_error)
}

async fn persist_hold(tx: &mut Transaction<'_, Postgres>, hold: &BookingPaymentHold) -> ApiResult<()> {
    sqlx::query("UPDATE booking_payment_holds SET captured_amount=$1, refunded_amount=$2, state=$3, updated_at=$4 WHERE id=$5 AND tenant_id=$6")
        .bind(hold.captured_amount).bind(hold.refunded_amount).bind(state_to_db(hold.state)).bind(hold.updated_at).bind(hold.id).bind(&hold.tenant_id)
        .execute(&mut **tx).await.map_err(db_error)?;
    Ok(())
}

async fn authorize(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<AuthorizeRequest>) -> ApiResult<(StatusCode, Json<HoldResponse>)> {
    let identity = authenticate(&state, &headers).await?;
    let key = idempotency_key(&headers)?;
    let fingerprint = format!("{}|{}|{}", req.booking_reference.trim(), req.amount, req.currency.trim().to_ascii_uppercase());
    let mut tx = state.db.begin().await.map_err(db_error)?;
    if let Some(response) = prior_response(&mut tx, &identity.tenant, &key, "authorize", &fingerprint).await? {
        tx.commit().await.map_err(db_error)?;
        return Ok((StatusCode::OK, Json(response)));
    }
    let hold = BookingPaymentHold::authorize(&identity.tenant, &req.booking_reference, req.amount, &req.currency, Utc::now()).map_err(lifecycle_error)?;
    sqlx::query("INSERT INTO booking_payment_holds (id,tenant_id,external_reference,booking_reference,amount,currency,captured_amount,refunded_amount,state,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(hold.id).bind(&hold.tenant_id).bind(&hold.external_reference).bind(&hold.booking_reference).bind(hold.amount).bind(&hold.currency)
        .bind(hold.captured_amount).bind(hold.refunded_amount).bind(state_to_db(hold.state)).bind(hold.created_at).bind(hold.updated_at)
        .execute(&mut *tx).await.map_err(|e| if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("23505") { api_error(StatusCode::CONFLICT, "BOOKING_PAYMENT_ALREADY_EXISTS", "A payment hold already exists for this booking") } else { db_error(e) })?;
    let response = hold_response(&hold, false);
    emit_event(&mut tx, &hold, "nvuto.payments.booking.hold.authorized.v1", &headers).await?;
    store_idempotency(&mut tx, &identity.tenant, &key, "authorize", &fingerprint, &response).await?;
    tx.commit().await.map_err(db_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn mutate_hold(
    state: Arc<AppState>, headers: HeaderMap, reference: String, operation: &'static str, refund_amount: Option<Decimal>,
) -> ApiResult<Json<HoldResponse>> {
    let identity = authenticate(&state, &headers).await?;
    let key = idempotency_key(&headers)?;
    let fingerprint = match refund_amount { Some(amount) => format!("{}|{}", reference, amount), None => reference.clone() };
    let mut tx = state.db.begin().await.map_err(db_error)?;
    if let Some(response) = prior_response(&mut tx, &identity.tenant, &key, operation, &fingerprint).await? {
        tx.commit().await.map_err(db_error)?;
        return Ok(Json(response));
    }
    let mut hold = load_hold_for_update(&mut tx, &identity.tenant, &reference).await?;
    let receipt: MutationReceipt = match operation {
        "capture" => hold.capture(Utc::now()),
        "release" => hold.release(Utc::now()),
        "refund" => hold.refund(refund_amount.ok_or_else(|| LifecycleError::InvalidAmount).map_err(lifecycle_error)?, Utc::now()),
        _ => Err(LifecycleError::InvalidState),
    }.map_err(lifecycle_error)?;
    persist_hold(&mut tx, &hold).await?;
    let event_type = match operation {
        "capture" => "nvuto.payments.booking.hold.captured.v1",
        "release" => "nvuto.payments.booking.hold.released.v1",
        "refund" => if hold.state == HoldState::Refunded { "nvuto.payments.booking.payment.refunded.v1" } else { "nvuto.payments.booking.payment.partially_refunded.v1" },
        _ => "nvuto.payments.booking.payment.changed.v1",
    };
    emit_event(&mut tx, &hold, event_type, &headers).await?;
    let mut response = hold_response(&hold, receipt.replayed);
    response.replayed = false;
    store_idempotency(&mut tx, &identity.tenant, &key, operation, &fingerprint, &response).await?;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(response))
}

async fn capture(State(state): State<Arc<AppState>>, Path(reference): Path<String>, headers: HeaderMap) -> ApiResult<Json<HoldResponse>> {
    mutate_hold(state, headers, reference, "capture", None).await
}

async fn release(State(state): State<Arc<AppState>>, Path(reference): Path<String>, headers: HeaderMap) -> ApiResult<Json<HoldResponse>> {
    mutate_hold(state, headers, reference, "release", None).await
}

async fn refund(State(state): State<Arc<AppState>>, Path(reference): Path<String>, headers: HeaderMap, Json(req): Json<RefundRequest>) -> ApiResult<Json<HoldResponse>> {
    mutate_hold(state, headers, reference, "refund", Some(req.amount)).await
}

async fn status(State(state): State<Arc<AppState>>, Path(reference): Path<String>, headers: HeaderMap) -> ApiResult<Json<HoldResponse>> {
    let identity = authenticate(&state, &headers).await?;
    let row = sqlx::query_as::<_, HoldRow>("SELECT * FROM booking_payment_holds WHERE tenant_id=$1 AND external_reference=$2")
        .bind(&identity.tenant).bind(reference).fetch_optional(&state.db).await.map_err(db_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "PAYMENT_NOT_FOUND", "Payment reference not found"))?;
    let hold: BookingPaymentHold = row.try_into().map_err(lifecycle_error)?;
    Ok(Json(hold_response(&hold, false)))
}

async fn health() -> Json<Value> {
    Json(json!({"status":"ok","service":"bookings-payments-api","contract_version":CONTRACT_VERSION}))
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL required")?;
    let identity_url = env::var("PLATFORM_IDENTITY_URL").context("PLATFORM_IDENTITY_URL required")?;
    let port: u16 = env::var("BOOKINGS_PAYMENTS_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8085);
    let db = PgPoolOptions::new().max_connections(20).connect(&database_url).await?;
    sqlx::migrate!("./migrations").run(&db).await?;
    let http = Client::builder().timeout(Duration::from_secs(3)).build()?;
    let state = Arc::new(AppState { db, identity_url, http });
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/bookings/payment-holds", post(authorize))
        .route("/api/v1/bookings/payment-holds/:reference", get(status))
        .route("/api/v1/bookings/payment-holds/:reference/capture", post(capture))
        .route("/api/v1/bookings/payment-holds/:reference/release", post(release))
        .route("/api/v1/bookings/payment-holds/:reference/refunds", post(refund))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
