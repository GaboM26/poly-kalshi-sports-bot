//! EIP-712 signing for CLOB authentication
//!
//! Implements the ClobAuth EIP-712 structured data signing.

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::signers::Signer;
use anyhow::Result;

/// CLOB domain name for EIP-712
pub const CLOB_DOMAIN_NAME: &str = "ClobAuthDomain";
/// CLOB version for EIP-712
pub const CLOB_VERSION: &str = "1";
/// Message to sign for authentication
pub const MSG_TO_SIGN: &str = "This message attests that I control the given wallet";

/// EIP-712 type hash for ClobAuth struct
/// keccak256("ClobAuth(address address,string timestamp,uint256 nonce,string message)")
const CLOB_AUTH_TYPE_HASH: &str =
    "5a46616a523f1aa5902f3a3b5deb3b79daf8963d8c46a6d0ced1c84408e51b96";

/// EIP-712 Domain separator struct
#[derive(Debug, Clone)]
pub struct ClobAuthDomain {
    pub name: String,
    pub version: String,
    pub chain_id: U256,
}

impl ClobAuthDomain {
    /// Create a new ClobAuth domain
    pub fn new(chain_id: u64) -> Self {
        Self {
            name: CLOB_DOMAIN_NAME.to_string(),
            version: CLOB_VERSION.to_string(),
            chain_id: U256::from(chain_id),
        }
    }

    /// Calculate domain separator hash
    pub fn domain_separator(&self) -> B256 {
        // EIP712Domain(string name,string version,uint256 chainId)
        let type_hash = keccak256(
            b"EIP712Domain(string name,string version,uint256 chainId)",
        );

        let name_hash = keccak256(self.name.as_bytes());
        let version_hash = keccak256(self.version.as_bytes());

        // Encode: type_hash + name_hash + version_hash + chain_id
        let mut data = Vec::with_capacity(128);
        data.extend_from_slice(type_hash.as_slice());
        data.extend_from_slice(name_hash.as_slice());
        data.extend_from_slice(version_hash.as_slice());
        data.extend_from_slice(&self.chain_id.to_be_bytes::<32>());

        keccak256(&data)
    }
}

/// ClobAuth struct for EIP-712 signing
#[derive(Debug, Clone)]
pub struct ClobAuth {
    pub address: Address,
    pub timestamp: String,
    pub nonce: U256,
    pub message: String,
}

impl ClobAuth {
    /// Create a new ClobAuth message
    pub fn new(address: Address, timestamp: i64, nonce: u64) -> Self {
        Self {
            address,
            timestamp: timestamp.to_string(),
            nonce: U256::from(nonce),
            message: MSG_TO_SIGN.to_string(),
        }
    }

    /// Calculate struct hash
    pub fn struct_hash(&self) -> B256 {
        let type_hash = B256::from_slice(&hex::decode(CLOB_AUTH_TYPE_HASH).unwrap());

        let timestamp_hash = keccak256(self.timestamp.as_bytes());
        let message_hash = keccak256(self.message.as_bytes());

        // Encode: type_hash + address + timestamp_hash + nonce + message_hash
        let mut data = Vec::with_capacity(160);
        data.extend_from_slice(type_hash.as_slice());
        // Address is padded to 32 bytes
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(self.address.as_slice());
        data.extend_from_slice(timestamp_hash.as_slice());
        data.extend_from_slice(&self.nonce.to_be_bytes::<32>());
        data.extend_from_slice(message_hash.as_slice());

        keccak256(&data)
    }

    /// Calculate the full EIP-712 hash to sign
    pub fn signable_hash(&self, domain: &ClobAuthDomain) -> B256 {
        let domain_separator = domain.domain_separator();
        let struct_hash = self.struct_hash();

        // "\x19\x01" + domain_separator + struct_hash
        let mut data = Vec::with_capacity(66);
        data.extend_from_slice(&[0x19, 0x01]);
        data.extend_from_slice(domain_separator.as_slice());
        data.extend_from_slice(struct_hash.as_slice());

        keccak256(&data)
    }
}

/// Sign a CLOB auth message
///
/// # Arguments
/// * `signer` - The wallet signer
/// * `timestamp` - Unix timestamp
/// * `nonce` - Nonce value
/// * `chain_id` - Chain ID
///
/// # Returns
/// Hex-encoded signature with 0x prefix
pub async fn sign_clob_auth_message<S: Signer>(
    signer: &S,
    timestamp: i64,
    nonce: u64,
    chain_id: u64,
) -> Result<String> {
    let address = signer.address();
    let clob_auth = ClobAuth::new(address, timestamp, nonce);
    let domain = ClobAuthDomain::new(chain_id);
    let hash = clob_auth.signable_hash(&domain);

    // Sign the hash
    let signature = signer.sign_hash(&hash).await?;

    // Return hex signature with 0x prefix
    Ok(format!("0x{}", hex::encode(signature.as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    #[test]
    fn test_domain_separator() {
        let domain = ClobAuthDomain::new(137);
        let separator = domain.domain_separator();
        // Domain separator should be deterministic
        assert!(!separator.is_zero());
    }

    #[test]
    fn test_struct_hash() {
        let auth = ClobAuth::new(
            address!("85B634AA874fd6a5E1a80ec9a64fDAbb395201D4"),
            1704067200,
            0,
        );
        let hash = auth.struct_hash();
        assert!(!hash.is_zero());
    }

    #[test]
    fn test_signable_hash() {
        let auth = ClobAuth::new(
            address!("85B634AA874fd6a5E1a80ec9a64fDAbb395201D4"),
            1704067200,
            0,
        );
        let domain = ClobAuthDomain::new(137);
        let hash = auth.signable_hash(&domain);
        assert!(!hash.is_zero());
    }
}
