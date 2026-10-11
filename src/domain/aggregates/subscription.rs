//! Subscription Aggregate
use crate::domain::events::{DomainEvent, SubscriptionEvent};
use crate::domain::value_objects::Money;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

#[derive(Clone, Debug)]
pub struct Subscription {
    id: String,
    customer_id: String,
    plan_id: String,
    status: SubscriptionStatus,
    current_period_start: NaiveDate,
    current_period_end: NaiveDate,
    billing_cycle: BillingCycle,
    amount: Money,
    cancel_at_period_end: bool,
    cancelled_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    events: Vec<DomainEvent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SubscriptionStatus {
    #[default]
    Active,
    PastDue,
    Cancelled,
    Trialing,
    Paused,
}

#[derive(Clone, Debug, Default)]
pub enum BillingCycle {
    #[default]
    Monthly,
    Yearly,
    Weekly,
}

impl Subscription {
    pub fn create(
        customer_id: impl Into<String>,
        plan_id: impl Into<String>,
        amount: Money,
        cycle: BillingCycle,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().date_naive();
        let period_end = match cycle {
            BillingCycle::Monthly => now + chrono::Duration::days(30),
            BillingCycle::Yearly => now + chrono::Duration::days(365),
            BillingCycle::Weekly => now + chrono::Duration::days(7),
        };
        let mut s = Self {
            id: id.clone(),
            customer_id: customer_id.into(),
            plan_id: plan_id.into(),
            status: SubscriptionStatus::Active,
            current_period_start: now,
            current_period_end: period_end,
            billing_cycle: cycle,
            amount,
            cancel_at_period_end: false,
            cancelled_at: None,
            created_at: Utc::now(),
            events: vec![],
        };
        s.raise_event(DomainEvent::Subscription(SubscriptionEvent::Created {
            subscription_id: id,
        }));
        s
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn status(&self) -> &SubscriptionStatus {
        &self.status
    }
    pub fn amount(&self) -> &Money {
        &self.amount
    }
    pub fn is_active(&self) -> bool {
        self.status == SubscriptionStatus::Active
    }

    pub fn renew(&mut self) {
        self.current_period_start = self.current_period_end;
        self.current_period_end = match self.billing_cycle {
            BillingCycle::Monthly => self.current_period_start + chrono::Duration::days(30),
            BillingCycle::Yearly => self.current_period_start + chrono::Duration::days(365),
            BillingCycle::Weekly => self.current_period_start + chrono::Duration::days(7),
        };
        self.raise_event(DomainEvent::Subscription(SubscriptionEvent::Renewed {
            subscription_id: self.id.clone(),
        }));
    }

    pub fn cancel(&mut self, at_period_end: bool) {
        if at_period_end {
            self.cancel_at_period_end = true;
        } else {
            self.status = SubscriptionStatus::Cancelled;
            self.cancelled_at = Some(Utc::now());
        }
        self.raise_event(DomainEvent::Subscription(SubscriptionEvent::Cancelled {
            subscription_id: self.id.clone(),
            at_period_end,
        }));
    }

    pub fn pause(&mut self) {
        self.status = SubscriptionStatus::Paused;
    }
    pub fn resume(&mut self) {
        if self.status == SubscriptionStatus::Paused {
            self.status = SubscriptionStatus::Active;
        }
    }

    pub fn take_events(&mut self) -> Vec<DomainEvent> {
        std::mem::take(&mut self.events)
    }
    fn raise_event(&mut self, e: DomainEvent) {
        self.events.push(e);
    }
}

#[derive(Debug, Clone)]
pub enum SubscriptionError {
    AlreadyCancelled,
    NotPaused,
}
impl std::error::Error for SubscriptionError {}
impl std::fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Subscription error")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_subscription() {
        let mut s = Subscription::create(
            "CUST001",
            "PLAN_PRO",
            Money::usd(Decimal::new(49, 0)),
            BillingCycle::Monthly,
        );
        assert!(s.is_active());
        s.cancel(true);
        assert!(s.cancel_at_period_end);
    }
}
