//! Authentication headers for CLOB API
//!
//! Creates Level 1 (EIP-712) and Level 2 (HMAC) authentication headers.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use super::signing::{build_hmac_signature, sign_clob_auth_message};
use super::signer::Signer;
use super::types::ApiCreds;

/// Header keys for Polymarket authentication
pub const POLY_ADDRESS: &str = "POLY_ADDRESS";
pub const POLY_SIGNATURE: &str = "POLY_SIGNATURE";
pub const POLY_TIMESTAMP: &str = "POLY_TIMESTAMP";
pub const POLY_NONCE: &str = "POLY_NONCE";
pub const POLY_API_KEY: &str = "POLY_API_KEY";
pub const POLY_PASSPHRASE: &str = "POLY_PASSPHRASE";

/// Get current Unix timestamp
fn get_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs() as i64
}

/// Create Level 1 authentication headers
///
/// Level 1 auth uses EIP-712 signatures for operations like:
/// - Creating API keys
/// - Deriving API keys
///
/// # Arguments
/// * `signer` - The wallet signer
/// * `nonce` - Optional nonce (defaults to 0)
///
/// # Returns
/// HashMap of header name -> value
pub async fn create_level_1_headers(
    signer: &Signer,
    nonce: Option<u64>,
) -> Result<HashMap<String, String>> {
    let timestamp = get_timestamp();
    let n = nonce.unwrap_or(0);

    // #region agent log
    let addr_debug = format!("{:?}", signer.address());
    let addr_checksum = signer.address().to_checksum(None);
    let log_data = serde_json::json!({"hypothesisId":"A","location":"headers.rs:create_level_1_headers","message":"Address formats","data":{"debug_format":addr_debug,"checksum_format":&addr_checksum,"timestamp":timestamp,"nonce":n},"timestamp":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),"sessionId":"debug-session"});
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/meloner/rustcode/polytaoli/.cursor/debug.log") { use std::io::Write; let _ = writeln!(f, "{}", log_data); }
    // #endregion

    let signature =
        sign_clob_auth_message(signer.inner(), timestamp, n, signer.chain_id()).await?;

    // #region agent log
    let log_data2 = serde_json::json!({"hypothesisId":"B","location":"headers.rs:create_level_1_headers","message":"Signature generated","data":{"signature":&signature,"timestamp":timestamp,"nonce":n,"chain_id":signer.chain_id()},"timestamp":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),"sessionId":"debug-session"});
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/meloner/rustcode/polytaoli/.cursor/debug.log") { use std::io::Write; let _ = writeln!(f, "{}", log_data2); }
    // #endregion

    let mut headers = HashMap::new();
    // FIX: Use checksum format instead of debug format for address
    headers.insert(POLY_ADDRESS.to_string(), signer.address().to_checksum(None));
    headers.insert(POLY_SIGNATURE.to_string(), signature);
    headers.insert(POLY_TIMESTAMP.to_string(), timestamp.to_string());
    headers.insert(POLY_NONCE.to_string(), n.to_string());

    // #region agent log
    let log_data3 = serde_json::json!({"hypothesisId":"A","location":"headers.rs:create_level_1_headers","message":"Headers created","data":{"POLY_ADDRESS":headers.get(POLY_ADDRESS),"POLY_TIMESTAMP":headers.get(POLY_TIMESTAMP),"POLY_NONCE":headers.get(POLY_NONCE),"runId":"post-fix"},"timestamp":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),"sessionId":"debug-session"});
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/meloner/rustcode/polytaoli/.cursor/debug.log") { use std::io::Write; let _ = writeln!(f, "{}", log_data3); }
    // #endregion

    Ok(headers)
}

/// Request arguments for Level 2 authentication
pub struct RequestArgs {
    pub method: String,
    pub request_path: String,
    pub body: Option<String>,
}

impl RequestArgs {
    pub fn new(method: &str, request_path: &str) -> Self {
        Self {
            method: method.to_string(),
            request_path: request_path.to_string(),
            body: None,
        }
    }

    pub fn with_body(mut self, body: &str) -> Self {
        self.body = Some(body.to_string());
        self
    }
}

/// Create Level 2 authentication headers
///
/// Level 2 auth uses HMAC signatures for authenticated operations like:
/// - Placing orders
/// - Canceling orders
/// - Getting account data
///
/// # Arguments
/// * `signer` - The wallet signer
/// * `creds` - API credentials
/// * `request_args` - Request method, path, and body
///
/// # Returns
/// HashMap of header name -> value
pub fn create_level_2_headers(
    signer: &Signer,
    creds: &ApiCreds,
    request_args: &RequestArgs,
) -> HashMap<String, String> {
    let timestamp = get_timestamp();

    let hmac_sig = build_hmac_signature(
        &creds.api_secret,
        &timestamp.to_string(),
        &request_args.method,
        &request_args.request_path,
        request_args.body.as_deref(),
    );

    let mut headers = HashMap::new();
    // FIX: Use checksum format instead of debug format for address
    headers.insert(POLY_ADDRESS.to_string(), signer.address().to_checksum(None));
    headers.insert(POLY_SIGNATURE.to_string(), hmac_sig);
    headers.insert(POLY_TIMESTAMP.to_string(), timestamp.to_string());
    headers.insert(POLY_API_KEY.to_string(), creds.api_key.clone());
    headers.insert(POLY_PASSPHRASE.to_string(), creds.api_passphrase.clone());

    headers
}

/// Convert header map to reqwest HeaderMap
pub fn to_header_map(
    headers: &HashMap<String, String>,
) -> Result<reqwest::header::HeaderMap> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    let mut header_map = HeaderMap::new();
    for (key, value) in headers {
        let name = HeaderName::from_bytes(key.as_bytes())?;
        let val = HeaderValue::from_str(value)?;
        header_map.insert(name, val);
    }
    Ok(header_map)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    #[tokio::test]
    async fn test_create_level_1_headers() {
        let signer = Signer::new(TEST_PRIVATE_KEY, 137).unwrap();
        let headers = create_level_1_headers(&signer, None).await.unwrap();

        assert!(headers.contains_key(POLY_ADDRESS));
        assert!(headers.contains_key(POLY_SIGNATURE));
        assert!(headers.contains_key(POLY_TIMESTAMP));
        assert!(headers.contains_key(POLY_NONCE));
    }

    #[test]
    fn test_create_level_2_headers() {
        let signer = Signer::new(TEST_PRIVATE_KEY, 137).unwrap();
        let creds = ApiCreds {
            api_key: "test_key".to_string(),
            api_secret: base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE,
                b"test_secret",
            ),
            api_passphrase: "test_passphrase".to_string(),
        };
        let request_args = RequestArgs::new("POST", "/order");

        let headers = create_level_2_headers(&signer, &creds, &request_args);

        assert!(headers.contains_key(POLY_ADDRESS));
        assert!(headers.contains_key(POLY_SIGNATURE));
        assert!(headers.contains_key(POLY_TIMESTAMP));
        assert!(headers.contains_key(POLY_API_KEY));
        assert!(headers.contains_key(POLY_PASSPHRASE));
    }
}
