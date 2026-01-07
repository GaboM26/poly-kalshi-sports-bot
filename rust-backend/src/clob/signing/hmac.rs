//! HMAC signing for CLOB Level 2 authentication
//!
//! Creates HMAC-SHA256 signatures for authenticated API requests.

use base64::{engine::general_purpose::URL_SAFE, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Build HMAC signature for a request
///
/// # Arguments
/// * `secret` - Base64 URL-safe encoded secret
/// * `timestamp` - Unix timestamp as string
/// * `method` - HTTP method (GET, POST, DELETE)
/// * `request_path` - Request path (e.g., "/order")
/// * `body` - Optional request body as JSON string
///
/// # Returns
/// Base64 URL-safe encoded HMAC signature
pub fn build_hmac_signature(
    secret: &str,
    timestamp: &str,
    method: &str,
    request_path: &str,
    body: Option<&str>,
) -> String {
    // Decode the base64 secret
    let decoded_secret = URL_SAFE
        .decode(secret)
        .expect("Failed to decode base64 secret");

    // Build the message to sign
    let mut message = format!("{}{}{}", timestamp, method, request_path);
    if let Some(body_str) = body {
        // NOTE: The body should already be properly serialized JSON
        // No need to replace quotes since Rust's serde_json produces valid JSON
        message.push_str(body_str);
    }

    // Create HMAC-SHA256
    let mut mac =
        HmacSha256::new_from_slice(&decoded_secret).expect("HMAC can take key of any size");
    mac.update(message.as_bytes());
    let result = mac.finalize();

    // Return base64 encoded signature
    URL_SAFE.encode(result.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_hmac_signature() {
        // Test secret (base64 encoded)
        let secret = URL_SAFE.encode(b"test_secret_key_12345");

        let timestamp = "1704067200";
        let method = "POST";
        let request_path = "/order";
        let body = Some(r#"{"token_id":"123","side":"BUY"}"#);

        let signature = build_hmac_signature(&secret, timestamp, method, request_path, body);

        // Signature should be non-empty base64 string
        assert!(!signature.is_empty());
        // Should be valid base64
        assert!(URL_SAFE.decode(&signature).is_ok());
    }

    #[test]
    fn test_signature_without_body() {
        let secret = URL_SAFE.encode(b"test_secret_key_12345");

        let timestamp = "1704067200";
        let method = "GET";
        let request_path = "/orders";

        let signature = build_hmac_signature(&secret, timestamp, method, request_path, None);

        assert!(!signature.is_empty());
    }

    #[test]
    fn test_signature_deterministic() {
        let secret = URL_SAFE.encode(b"test_secret_key_12345");
        let timestamp = "1704067200";
        let method = "POST";
        let request_path = "/order";
        let body = Some(r#"{"key":"value"}"#);

        let sig1 = build_hmac_signature(&secret, timestamp, method, request_path, body);
        let sig2 = build_hmac_signature(&secret, timestamp, method, request_path, body);

        assert_eq!(sig1, sig2);
    }
}
