//! Optional jshift-backed in-place JSON field mutation helpers.
//!
//! Enabled with the `jshift` Cargo feature. Not used by the core indexing path;
//! provided for tooling that streams JSONL manifests or agent side-channels
//! without full `serde_json` re-serialization.
//!
//! **Why optional / not default:** criterion benches on rustbrain-shaped workloads
//! show `serde_json` wins for full typed encode/decode and pretty CLI JSON; jshift
//! wins for sparse path get and in-place patch on larger buffers. See
//! `docs/JSON_STACK.md` in the repository root.

#[cfg(feature = "jshift")]
mod imp {
    use crate::error::{BrainError, Result};
    use jshift::{find_value, mutate_value, parse_path};

    /// Mutate a JSON byte buffer in-place using jshift.
    pub fn update_json_field_inplace(
        json_bytes: &mut Vec<u8>,
        path_expr: &str,
        new_value_bytes: &[u8],
    ) -> Result<()> {
        let path = parse_path(path_expr);
        mutate_value(json_bytes, &path, new_value_bytes)
            .map_err(|e| BrainError::other(format!("jshift mutate failed: {e}")))?;
        Ok(())
    }

    /// Zero-copy extraction of a JSON field slice from raw bytes.
    pub fn extract_json_field_slice<'a>(
        json_bytes: &'a [u8],
        path_expr: &str,
    ) -> Option<&'a [u8]> {
        let path = parse_path(path_expr);
        find_value(json_bytes, &path).ok()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn jshift_inplace_mutation() {
            let mut json =
                b"{\"node_id\": \"concept-1\", \"status\": \"draft\", \"weight\": 0.5}".to_vec();
            update_json_field_inplace(&mut json, "status", b"\"published\"").unwrap();
            let status = extract_json_field_slice(&json, "status").unwrap();
            assert_eq!(status, b"\"published\"");
        }
    }
}

#[cfg(feature = "jshift")]
pub use imp::*;
