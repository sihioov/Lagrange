//! Read-only client for the one owner-approved FSC public-data surface.
//!
//! The only outbound request this crate can construct is:
//!
//! `GET https://apis.data.go.kr/1160100/service/GetKrxListedInfoService/getItemInfo`
//!
//! `serviceKey` is loaded from a protected file and appended inside the private
//! transport boundary. It is never part of [`ItemInfoQuery`], public request
//! metadata, diagnostics, or any `Debug` implementation. `resultType=json` is
//! fixed in the visible query so the portal cannot fall back to XML. No
//! response parsing happens here; the market-data Raw adapter owns the
//! documented JSON contract.

mod client;
mod credential;
mod error;
mod transport;

pub use client::{ClientConfig, DataGoClient, ITEM_INFO_PAGE_SIZE, ItemInfoQuery};
pub use credential::{
    CredentialError, CredentialRef, CredentialSource, MAX_SERVICE_KEY_BYTES,
    SERVICE_KEY_FILE_ENV_VAR, Secret, SystemCredentialSource,
};
pub use error::DataGoTransportError;

/// The only host accepted by this crate.
pub const DATA_GO_BASE_URL: &str = "https://apis.data.go.kr";
/// The only path accepted by this crate.
pub const KRX_LISTED_ITEM_INFO_PATH: &str = "/1160100/service/GetKrxListedInfoService/getItemInfo";
/// The exact endpoint identifier used by the Raw request metadata.
pub const KRX_LISTED_ITEM_INFO_ENDPOINT: &str =
    "https://apis.data.go.kr/1160100/service/GetKrxListedInfoService/getItemInfo";

/// The only ISIN values accepted by the low-level client. Keeping this
/// allowlist here prevents a future caller from widening the provider scope by
/// constructing a direct client query.
pub const APPROVED_FIXED_ETF11_ISINS: [&str; 11] = [
    "KR7069500007",
    "KR7102110004",
    "KR7229200001",
    "KR7143850006",
    "KR7133690008",
    "KR7195930003",
    "KR7192090009",
    "KR7148070006",
    "KR7114260003",
    "KR7153130000",
    "KR7132030008",
];

/// Maximum response bytes admitted by the live transport before JSON parsing
/// or Raw storage can allocate based on the provider body.
pub const MAX_RESPONSE_BODY_BYTES: usize = 512 * 1024;

/// The sole path allowlist. Keep this deny-by-default and do not add adjacent
/// public-data or KRX operations without a new owner approval.
pub const ALLOWED_PATHS: [&str; 1] = [KRX_LISTED_ITEM_INFO_PATH];

/// Resolves only the one allowlisted path.
pub fn resolve_path(path: &str) -> Option<&'static str> {
    ALLOWED_PATHS
        .iter()
        .find(|&&candidate| candidate == path)
        .copied()
}
