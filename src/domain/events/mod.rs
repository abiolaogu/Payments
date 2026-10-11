//! Payment domain events
use crate::domain::value_objects::PaymentId;
use rust_decimal::Decimal;

#[derive(Clone, Debug)]
pub enum DomainEvent {
    Payment(PaymentEvent),
    Subscription(SubscriptionEvent),
}

#[derive(Clone, Debug)]
pub enum PaymentEvent {
    Created {
        payment_id: PaymentId,
        amount: Decimal,
    },
    Succeeded {
        payment_id: PaymentId,
    },
    Failed {
        payment_id: PaymentId,
        reason: String,
    },
    Refunded {
        payment_id: PaymentId,
        amount: Decimal,
    },
}

#[derive(Clone, Debug)]
pub enum SubscriptionEvent {
    Created {
        subscription_id: String,
    },
    Renewed {
        subscription_id: String,
    },
    Cancelled {
        subscription_id: String,
        at_period_end: bool,
    },
    PaymentFailed {
        subscription_id: String,
    },
}
