//! Payment value objects
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaymentId(String);
impl PaymentId {
    pub fn new() -> Self {
        Self(format!(
            "pay_{}",
            uuid::Uuid::new_v4().to_string().replace("-", "")[..24].to_string()
        ))
    }
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl Default for PaymentId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for PaymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaymentMethod {
    pub method_type: PaymentMethodType,
    pub last_four: Option<String>,
    pub brand: Option<String>,
    pub exp_month: Option<u8>,
    pub exp_year: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PaymentMethodType {
    Card,
    BankTransfer,
    Wallet,
    Crypto,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Money {
    pub amount: rust_decimal::Decimal,
    pub currency: String,
}
impl Money {
    pub fn new(amount: rust_decimal::Decimal, currency: &str) -> Self {
        Self {
            amount,
            currency: currency.to_string(),
        }
    }
    pub fn usd(amount: rust_decimal::Decimal) -> Self {
        Self::new(amount, "USD")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_payment_id() {
        let id = PaymentId::new();
        assert!(id.as_str().starts_with("pay_"));
    }
}
