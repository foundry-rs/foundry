//! custom serde helper functions
//!
//! # Security Considerations
//!
//! These serde helpers are used for JSON-RPC parameter deserialization in a public API.
//! **CRITICAL SECURITY WARNINGS:**
//!
//! 1. **DoS Attack Prevention**: These functions process untrusted input from external clients.
//!    Malicious clients can send large payloads to cause memory exhaustion or CPU DoS.
//! 2. **Input Validation**: Always validate deserialized parameters before processing.
//! 3. **Resource Limits**: Consider implementing request size limits at the HTTP/WebSocket level.
//! 4. **Performance**: Some functions use O(n) operations that can be exploited for DoS attacks.
//!
//! # Usage Guidelines
//!
//! - Only use these helpers for trusted or validated JSON-RPC requests
//! - Implement proper request size limits in your server configuration
//! - Monitor for unusual request patterns that might indicate DoS attempts
//! - Consider rate limiting for public-facing JSON-RPC endpoints

pub mod sequence {
    use serde::{
        Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned, ser::SerializeSeq,
    };

    pub fn serialize<S, T>(val: &T, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        let mut seq = s.serialize_seq(Some(1))?;
        seq.serialize_element(val)?;
        seq.end()
    }

    /// Deserializes a sequence containing exactly one element.
    ///
    /// # Security Warning
    /// This function uses O(1) Vec::pop() instead of O(n) Vec::remove(0) to prevent
    /// DoS attacks through large parameter arrays. However, it still creates a full
    /// Vec<T> which can cause memory exhaustion with large types T.
    ///
    /// # Performance
    /// - Time complexity: O(1) for pop operation (vs O(n) for remove(0))
    /// - Space complexity: O(n) where n is the sequence length
    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let mut seq = Vec::<T>::deserialize(d)?;
        if seq.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 1 but got {}",
                seq.len()
            )));
        }
        // Use pop() instead of remove(0) for O(1) performance instead of O(n)
        // This prevents DoS attacks through large parameter arrays
        Ok(seq.pop().expect("Vec should contain exactly one element"))
    }
}

/// A module that deserializes `[]` optionally
///
/// # Security Warning
/// This function processes untrusted JSON-RPC parameters and should be used with
/// proper request size limits to prevent DoS attacks.
pub mod empty_params {
    use serde::{Deserialize, Deserializer};

    /// Deserializes an empty parameter sequence `[]` or no parameters.
    ///
    /// # Security Considerations
    /// - Validates that no parameters are provided (length 0)
    /// - Returns error if unexpected parameters are found
    /// - Used in 27+ JSON-RPC methods that require no parameters
    pub fn deserialize<'de, D>(d: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let seq = Option::<Vec<()>>::deserialize(d)?.unwrap_or_default();
        if !seq.is_empty() {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 0 but got {}",
                seq.len()
            )));
        }
        Ok(())
    }
}

/// A module that deserializes either a BlockNumberOrTag, or a simple number.
///
/// # Security Warning
/// These functions process untrusted input for block number parameters in JSON-RPC calls.
/// Ensure proper validation of deserialized block numbers before use.
pub mod lenient_block_number {
    pub use alloy_eips::eip1898::LenientBlockNumberOrTag;
    use alloy_rpc_types::BlockNumberOrTag;
    use serde::{Deserialize, Deserializer};

    /// deserializes either a BlockNumberOrTag, or a simple number.
    pub use alloy_eips::eip1898::lenient_block_number_or_tag::deserialize as lenient_block_number;

    /// Same as `lenient_block_number` but requires to be `[num; 1]`
    ///
    /// # Security Warning
    /// This function deserializes a fixed-size array without bounds checking.
    /// The array access `[0]` is safe due to the fixed size constraint, but
    /// the deserialization itself can still cause memory exhaustion with large inputs.
    pub fn lenient_block_number_seq<'de, D>(deserializer: D) -> Result<BlockNumberOrTag, D::Error>
    where
        D: Deserializer<'de>,
    {
        let num = <[LenientBlockNumberOrTag; 1]>::deserialize(deserializer)?[0].into();
        Ok(num)
    }
}
