//! Active immutable Phase 0 dataset contract.

/// Dataset identity carried by active Phase 0 requests and results.
pub const DATASET_ID: &str = "kr-etf-daily-phase0-v2";
/// Curated partition version read by active Phase 0 consumers.
pub const CURATED_VERSION: u32 = 2;

#[cfg(test)]
mod tests {
    use super::{CURATED_VERSION, DATASET_ID};

    #[test]
    fn active_phase0_contract_is_the_corrected_v2_dataset() {
        assert_eq!(DATASET_ID, "kr-etf-daily-phase0-v2");
        assert_eq!(CURATED_VERSION, 2);
    }
}
