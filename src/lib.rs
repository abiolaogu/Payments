//! OpenSASE Payments Platform - DDD Implementation
//!
//! Self-hosted payment gateway, Stripe alternative.

pub mod bookings_contract;
pub mod domain;
pub mod security;

pub use bookings_contract::{
    validate_idempotency, BookingPaymentHold, HoldState, IdempotencyRecord, LifecycleError,
    MutationReceipt, CONTRACT_VERSION,
};
pub use domain::aggregates::{Payment, PaymentError, Subscription, SubscriptionError};
pub use domain::events::{DomainEvent, PaymentEvent, SubscriptionEvent};
pub use domain::value_objects::{PaymentId, PaymentMethod};
