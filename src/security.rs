//! Fail-closed identity and payment-provider verification primitives.

use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha512;
use subtle::ConstantTimeEq;

type HmacSha512 = Hmac<Sha512>;

pub const LEGACY_ADMIN_PERMISSION: &str = "payments:legacy-admin";

#[derive(Debug, Clone, Deserialize)]
pub struct Introspection {
    pub active: bool,
    #[serde(rename = "sub")]
    pub subject: String,
    pub tenant: String,
    #[serde(default)]
    pub exp: i64,
    #[serde(default)]
    pub permissions: Vec<String>,
}

impl Introspection {
    pub fn authorizes_legacy_admin(&self) -> bool {
        self.active
            && !self.subject.trim().is_empty()
            && !self.tenant.trim().is_empty()
            && (self.exp == 0 || self.exp > Utc::now().timestamp())
            && self
                .permissions
                .iter()
                .any(|value| value == LEGACY_ADMIN_PERMISSION)
    }
}

pub fn bearer(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookProvider {
    Paystack,
    Flutterwave,
}

pub fn verify_provider_webhook(
    payload: &[u8],
    paystack_signature: Option<&str>,
    flutterwave_hash: Option<&str>,
    paystack_secret: Option<&str>,
    flutterwave_secret: Option<&str>,
) -> Option<WebhookProvider> {
    if let (Some(signature), Some(secret)) = (paystack_signature, paystack_secret) {
        if let (Ok(signature), Ok(mut mac)) = (
            hex::decode(signature.trim()),
            HmacSha512::new_from_slice(secret.as_bytes()),
        ) {
            mac.update(payload);
            if mac.verify_slice(&signature).is_ok() {
                return Some(WebhookProvider::Paystack);
            }
        }
    }

    if let (Some(hash), Some(secret)) = (flutterwave_hash, flutterwave_secret) {
        if hash.trim().as_bytes().ct_eq(secret.as_bytes()).into() {
            return Some(WebhookProvider::Flutterwave);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_requires_an_explicit_non_empty_scheme() {
        assert_eq!(
            bearer(Some("Bearer workload-token")),
            Some("workload-token")
        );
        assert_eq!(bearer(Some("bearer lower-case")), Some("lower-case"));
        assert_eq!(bearer(Some("workload-token")), None);
        assert_eq!(bearer(Some("Bearer   ")), None);
        assert_eq!(bearer(None), None);
    }

    #[test]
    fn legacy_api_requires_active_tenant_bound_admin_permission() {
        let authorized = Introspection {
            active: true,
            subject: "workload-1".into(),
            tenant: "tenant-1".into(),
            exp: Utc::now().timestamp() + 60,
            permissions: vec![LEGACY_ADMIN_PERMISSION.into()],
        };
        assert!(authorized.authorizes_legacy_admin());

        let mut missing_permission = authorized.clone();
        missing_permission.permissions.clear();
        assert!(!missing_permission.authorizes_legacy_admin());

        let mut expired = authorized;
        expired.exp = Utc::now().timestamp() - 1;
        assert!(!expired.authorizes_legacy_admin());
    }

    #[test]
    fn paystack_webhooks_require_a_valid_body_signature() {
        let payload = br#"{"event":"charge.success"}"#;
        let mut mac = HmacSha512::new_from_slice(b"provider-secret").unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());

        assert_eq!(
            verify_provider_webhook(
                payload,
                Some(&signature),
                None,
                Some("provider-secret"),
                None,
            ),
            Some(WebhookProvider::Paystack)
        );
        assert_eq!(
            verify_provider_webhook(
                br#"{"event":"tampered"}"#,
                Some(&signature),
                None,
                Some("provider-secret"),
                None,
            ),
            None
        );
        assert_eq!(
            verify_provider_webhook(payload, None, None, Some("provider-secret"), None),
            None
        );
    }

    #[test]
    fn flutterwave_webhooks_require_constant_time_secret_match() {
        assert_eq!(
            verify_provider_webhook(
                br#"{"event":"charge.completed"}"#,
                None,
                Some("provider-secret"),
                None,
                Some("provider-secret"),
            ),
            Some(WebhookProvider::Flutterwave)
        );
        assert_eq!(
            verify_provider_webhook(
                br#"{"event":"charge.completed"}"#,
                None,
                Some("wrong-secret"),
                None,
                Some("provider-secret"),
            ),
            None
        );
    }
}
