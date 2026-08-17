use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const CONTRACT_VERSION: &str = "nvuto.payments.booking.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HoldState {
    Authorized,
    Captured,
    Released,
    PartiallyRefunded,
    Refunded,
    Declined,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookingPaymentHold {
    pub id: Uuid,
    pub tenant_id: String,
    pub external_reference: String,
    pub booking_reference: String,
    pub amount: Decimal,
    pub currency: String,
    pub captured_amount: Decimal,
    pub refunded_amount: Decimal,
    pub state: HoldState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationReceipt {
    pub external_reference: String,
    pub state: HoldState,
    pub amount: Decimal,
    pub replayed: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("tenant is required")]
    MissingTenant,
    #[error("booking reference is required")]
    MissingBookingReference,
    #[error("idempotency key is required")]
    MissingIdempotencyKey,
    #[error("amount must be positive")]
    InvalidAmount,
    #[error("currency must be a three-letter ISO code")]
    InvalidCurrency,
    #[error("payment hold has already been captured")]
    AlreadyCaptured,
    #[error("payment hold has already been released")]
    AlreadyReleased,
    #[error("payment hold has already been fully refunded")]
    AlreadyRefunded,
    #[error("payment authorization was declined")]
    Declined,
    #[error("payment authorization has expired")]
    Expired,
    #[error("refund exceeds captured balance")]
    RefundExceedsCaptured,
    #[error("operation is not valid in the current payment state")]
    InvalidState,
    #[error("idempotency key was reused with a different operation or payload")]
    IdempotencyConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyRecord {
    pub key: String,
    pub fingerprint: String,
    pub operation: String,
}

impl BookingPaymentHold {
    pub fn authorize(
        tenant_id: impl Into<String>,
        booking_reference: impl Into<String>,
        amount: Decimal,
        currency: impl Into<String>,
        now: DateTime<Utc>,
    ) -> Result<Self, LifecycleError> {
        let tenant_id = tenant_id.into().trim().to_string();
        let booking_reference = booking_reference.into().trim().to_string();
        let currency = currency.into().trim().to_ascii_uppercase();
        if tenant_id.is_empty() { return Err(LifecycleError::MissingTenant); }
        if booking_reference.is_empty() { return Err(LifecycleError::MissingBookingReference); }
        if amount <= Decimal::ZERO { return Err(LifecycleError::InvalidAmount); }
        if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(LifecycleError::InvalidCurrency);
        }
        let id = Uuid::now_v7();
        Ok(Self {
            id,
            tenant_id,
            external_reference: format!("NVH-{}", id),
            booking_reference,
            amount,
            currency,
            captured_amount: Decimal::ZERO,
            refunded_amount: Decimal::ZERO,
            state: HoldState::Authorized,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn capture(&mut self, now: DateTime<Utc>) -> Result<MutationReceipt, LifecycleError> {
        match self.state {
            HoldState::Authorized => {
                self.captured_amount = self.amount;
                self.state = HoldState::Captured;
                self.updated_at = now;
                Ok(self.receipt(self.amount, false))
            }
            HoldState::Captured | HoldState::PartiallyRefunded | HoldState::Refunded => Err(LifecycleError::AlreadyCaptured),
            HoldState::Released => Err(LifecycleError::AlreadyReleased),
            HoldState::Declined => Err(LifecycleError::Declined),
            HoldState::Expired => Err(LifecycleError::Expired),
        }
    }

    pub fn release(&mut self, now: DateTime<Utc>) -> Result<MutationReceipt, LifecycleError> {
        match self.state {
            HoldState::Authorized => {
                self.state = HoldState::Released;
                self.updated_at = now;
                Ok(self.receipt(self.amount, false))
            }
            HoldState::Released => Err(LifecycleError::AlreadyReleased),
            HoldState::Captured | HoldState::PartiallyRefunded | HoldState::Refunded => Err(LifecycleError::AlreadyCaptured),
            HoldState::Declined => Err(LifecycleError::Declined),
            HoldState::Expired => Err(LifecycleError::Expired),
        }
    }

    pub fn refund(&mut self, amount: Decimal, now: DateTime<Utc>) -> Result<MutationReceipt, LifecycleError> {
        if amount <= Decimal::ZERO { return Err(LifecycleError::InvalidAmount); }
        match self.state {
            HoldState::Refunded => return Err(LifecycleError::AlreadyRefunded),
            HoldState::Captured | HoldState::PartiallyRefunded => {}
            HoldState::Declined => return Err(LifecycleError::Declined),
            HoldState::Expired => return Err(LifecycleError::Expired),
            HoldState::Authorized | HoldState::Released => return Err(LifecycleError::InvalidState),
        }
        let refundable = self.captured_amount - self.refunded_amount;
        if amount > refundable { return Err(LifecycleError::RefundExceedsCaptured); }
        self.refunded_amount += amount;
        self.state = if self.refunded_amount == self.captured_amount { HoldState::Refunded } else { HoldState::PartiallyRefunded };
        self.updated_at = now;
        Ok(self.receipt(amount, false))
    }

    pub fn receipt(&self, amount: Decimal, replayed: bool) -> MutationReceipt {
        MutationReceipt { external_reference: self.external_reference.clone(), state: self.state, amount, replayed }
    }
}

pub fn validate_idempotency(key: &str, operation: &str, fingerprint: &str, prior: Option<&IdempotencyRecord>) -> Result<bool, LifecycleError> {
    let key = key.trim();
    if key.is_empty() { return Err(LifecycleError::MissingIdempotencyKey); }
    if let Some(prior) = prior {
        if prior.key == key {
            if prior.operation == operation && prior.fingerprint == fingerprint { return Ok(true); }
            return Err(LifecycleError::IdempotencyConflict);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> { Utc.with_ymd_and_hms(2026, 8, 16, 22, 0, 0).unwrap() }

    #[test]
    fn lifecycle_authorize_capture_refund_is_bounded() {
        let mut hold = BookingPaymentHold::authorize("tenant-a", "booking-1", Decimal::new(25000, 2), "ngn", now()).unwrap();
        assert_eq!(hold.state, HoldState::Authorized);
        hold.capture(now()).unwrap();
        assert_eq!(hold.state, HoldState::Captured);
        hold.refund(Decimal::new(10000, 2), now()).unwrap();
        assert_eq!(hold.state, HoldState::PartiallyRefunded);
        hold.refund(Decimal::new(15000, 2), now()).unwrap();
        assert_eq!(hold.state, HoldState::Refunded);
        assert_eq!(hold.refunded_amount, hold.captured_amount);
    }

    #[test]
    fn released_hold_cannot_be_captured() {
        let mut hold = BookingPaymentHold::authorize("tenant-a", "booking-1", Decimal::new(1000, 2), "USD", now()).unwrap();
        hold.release(now()).unwrap();
        assert_eq!(hold.capture(now()), Err(LifecycleError::AlreadyReleased));
    }

    #[test]
    fn refund_cannot_exceed_capture() {
        let mut hold = BookingPaymentHold::authorize("tenant-a", "booking-1", Decimal::new(1000, 2), "USD", now()).unwrap();
        hold.capture(now()).unwrap();
        assert_eq!(hold.refund(Decimal::new(1001, 2), now()), Err(LifecycleError::RefundExceedsCaptured));
    }

    #[test]
    fn declined_and_expired_are_terminal_for_capture() {
        let mut declined = BookingPaymentHold::authorize("tenant-a", "booking-1", Decimal::new(1000, 2), "USD", now()).unwrap();
        declined.state = HoldState::Declined;
        assert_eq!(declined.capture(now()), Err(LifecycleError::Declined));
        let mut expired = BookingPaymentHold::authorize("tenant-a", "booking-2", Decimal::new(1000, 2), "USD", now()).unwrap();
        expired.state = HoldState::Expired;
        assert_eq!(expired.capture(now()), Err(LifecycleError::Expired));
    }

    #[test]
    fn idempotency_replay_requires_same_operation_and_fingerprint() {
        let prior = IdempotencyRecord { key: "idem-1".into(), operation: "capture".into(), fingerprint: "hold-1".into() };
        assert_eq!(validate_idempotency("idem-1", "capture", "hold-1", Some(&prior)), Ok(true));
        assert_eq!(validate_idempotency("idem-1", "release", "hold-1", Some(&prior)), Err(LifecycleError::IdempotencyConflict));
        assert_eq!(validate_idempotency("idem-1", "capture", "hold-2", Some(&prior)), Err(LifecycleError::IdempotencyConflict));
    }
}
