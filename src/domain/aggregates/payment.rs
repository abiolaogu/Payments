//! Payment Aggregate
use crate::domain::events::{DomainEvent, PaymentEvent};
use crate::domain::value_objects::{Money, PaymentId, PaymentMethod};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

#[derive(Clone, Debug)]
pub struct Payment {
    id: PaymentId,
    customer_id: String,
    amount: Money,
    status: PaymentStatus,
    payment_method: Option<PaymentMethod>,
    description: Option<String>,
    metadata: std::collections::HashMap<String, String>,
    refunded_amount: Decimal,
    created_at: DateTime<Utc>,
    events: Vec<DomainEvent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PaymentStatus {
    #[default]
    Pending,
    Processing,
    Succeeded,
    Failed,
    Cancelled,
    Refunded,
    PartiallyRefunded,
}

impl Payment {
    pub fn create(customer_id: impl Into<String>, amount: Money) -> Self {
        let id = PaymentId::new();
        let mut p = Self {
            id: id.clone(),
            customer_id: customer_id.into(),
            amount: amount.clone(),
            status: PaymentStatus::Pending,
            payment_method: None,
            description: None,
            metadata: std::collections::HashMap::new(),
            refunded_amount: Decimal::ZERO,
            created_at: Utc::now(),
            events: vec![],
        };
        p.raise_event(DomainEvent::Payment(PaymentEvent::Created {
            payment_id: id,
            amount: amount.amount,
        }));
        p
    }

    pub fn id(&self) -> &PaymentId {
        &self.id
    }
    pub fn amount(&self) -> &Money {
        &self.amount
    }
    pub fn status(&self) -> &PaymentStatus {
        &self.status
    }

    pub fn process(&mut self, method: PaymentMethod) -> Result<(), PaymentError> {
        if self.status != PaymentStatus::Pending {
            return Err(PaymentError::InvalidStatus);
        }
        self.payment_method = Some(method);
        self.status = PaymentStatus::Processing;
        Ok(())
    }

    pub fn succeed(&mut self) -> Result<(), PaymentError> {
        if self.status != PaymentStatus::Processing {
            return Err(PaymentError::InvalidStatus);
        }
        self.status = PaymentStatus::Succeeded;
        self.raise_event(DomainEvent::Payment(PaymentEvent::Succeeded {
            payment_id: self.id.clone(),
        }));
        Ok(())
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        self.status = PaymentStatus::Failed;
    }

    pub fn refund(&mut self, amount: Decimal) -> Result<(), PaymentError> {
        if self.status != PaymentStatus::Succeeded
            && self.status != PaymentStatus::PartiallyRefunded
        {
            return Err(PaymentError::NotRefundable);
        }
        let new_total = self.refunded_amount + amount;
        if new_total > self.amount.amount {
            return Err(PaymentError::RefundExceedsPayment);
        }
        self.refunded_amount = new_total;
        self.status = if new_total == self.amount.amount {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };
        self.raise_event(DomainEvent::Payment(PaymentEvent::Refunded {
            payment_id: self.id.clone(),
            amount,
        }));
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<DomainEvent> {
        std::mem::take(&mut self.events)
    }
    fn raise_event(&mut self, e: DomainEvent) {
        self.events.push(e);
    }
}

#[derive(Debug, Clone)]
pub enum PaymentError {
    InvalidStatus,
    NotRefundable,
    RefundExceedsPayment,
}
impl std::error::Error for PaymentError {}
impl std::fmt::Display for PaymentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStatus => write!(f, "Invalid status"),
            Self::NotRefundable => write!(f, "Not refundable"),
            Self::RefundExceedsPayment => write!(f, "Refund exceeds payment"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_payment_workflow() {
        let mut p = Payment::create("CUST001", Money::usd(Decimal::new(100, 0)));
        p.process(PaymentMethod {
            method_type: crate::domain::value_objects::PaymentMethodType::Card,
            last_four: Some("4242".into()),
            brand: Some("Visa".into()),
            exp_month: Some(12),
            exp_year: Some(2025),
        })
        .unwrap();
        p.succeed().unwrap();
        assert_eq!(p.status(), &PaymentStatus::Succeeded);
    }
}
