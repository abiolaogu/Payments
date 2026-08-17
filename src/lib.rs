//! OpenSASE Payments Platform - DDD Implementation
//!
//! Self-hosted payment gateway, Stripe alternative.

pub mod bookings_contract;
pub mod domain;

pub use bookings_contract::{
    validate_idempotency, BookingPaymentHold, HoldState, IdempotencyRecord, LifecycleError,
    MutationReceipt, CONTRACT_VERSION,
};
pub use domain::aggregates::{Payment, Subscription, PaymentError, SubscriptionError};
pub use domain::value_objects::{PaymentId, PaymentMethod};
pub use domain::events::{DomainEvent, PaymentEvent, SubscriptionEvent};
