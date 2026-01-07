//! Ethereum signer for CLOB operations
//!
//! Wraps alloy's LocalWallet for signing operations.

use alloy::primitives::{Address, B256};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer as AlloySigner;
use anyhow::{Context, Result};

/// Signer for Ethereum operations
#[derive(Clone)]
pub struct Signer {
    wallet: PrivateKeySigner,
    chain_id: u64,
}

impl Signer {
    /// Create a new signer from a private key
    ///
    /// # Arguments
    /// * `private_key` - Hex-encoded private key (with or without 0x prefix)
    /// * `chain_id` - Ethereum chain ID
    pub fn new(private_key: &str, chain_id: u64) -> Result<Self> {
        // Remove 0x prefix if present
        let key_hex = private_key.strip_prefix("0x").unwrap_or(private_key);

        let wallet: PrivateKeySigner = key_hex
            .parse()
            .with_context(|| "Failed to parse private key")?;

        Ok(Self { wallet, chain_id })
    }

    /// Get the signer's address
    pub fn address(&self) -> Address {
        self.wallet.address()
    }

    /// Get the chain ID
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Sign a message hash
    ///
    /// # Arguments
    /// * `hash` - 32-byte message hash to sign
    ///
    /// # Returns
    /// Hex-encoded signature with 0x prefix
    pub async fn sign_hash(&self, hash: &B256) -> Result<String> {
        let signature = self
            .wallet
            .sign_hash(hash)
            .await
            .with_context(|| "Failed to sign hash")?;

        Ok(format!("0x{}", hex::encode(signature.as_bytes())))
    }

    /// Get the inner wallet for direct signing operations
    pub fn inner(&self) -> &PrivateKeySigner {
        &self.wallet
    }
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("address", &self.address())
            .field("chain_id", &self.chain_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test private key (DO NOT USE IN PRODUCTION)
    const TEST_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    #[test]
    fn test_signer_creation() {
        let signer = Signer::new(TEST_PRIVATE_KEY, 137).unwrap();
        assert_eq!(signer.chain_id(), 137);
        // Address should be derived deterministically
        assert!(!signer.address().is_zero());
    }

    #[test]
    fn test_signer_without_0x_prefix() {
        let key_without_prefix =
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let signer = Signer::new(key_without_prefix, 137).unwrap();
        assert!(!signer.address().is_zero());
    }

    #[tokio::test]
    async fn test_sign_hash() {
        let signer = Signer::new(TEST_PRIVATE_KEY, 137).unwrap();
        let hash = B256::from([1u8; 32]);
        let signature = signer.sign_hash(&hash).await.unwrap();

        // Signature should start with 0x
        assert!(signature.starts_with("0x"));
        // Signature should be 65 bytes (130 hex chars + 2 for 0x)
        assert_eq!(signature.len(), 132);
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    
    #[test]
    fn test_address_format() {
        let signer = Signer::new("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80", 137).unwrap();
        let addr = signer.address();
        println!("Debug format: {:?}", addr);
        println!("Display format: {}", addr);
        println!("to_checksum: {}", addr.to_checksum(None));
    }
}
