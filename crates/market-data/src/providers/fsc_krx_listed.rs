//! Raw-only adapter for 금융위원회 `KRX상장종목정보`.
//!
//! This is a deliberately separate source contract. It is not the KIS EOD
//! reference path, not a disclosure source, and not a general listing-master
//! publication. The adapter issues one exact `basDt` + `isinCd` query for each
//! member of the fixed ETF11 universe, validates the documented paginated JSON
//! envelope, and stores the original response bytes only after all eleven
//! identities have been proven present.
//!
//! The response kind is [`ResponseKind::CandidateMaster`] because that kind is
//! already excluded from the EOD/candidate publication registries. The source
//! is an identity master, but it must not be routed through the KIS fixed-width
//! candidate-master parser or any generic EOD validator.

use std::future::Future;

use data_go_client::{
    DataGoClient, DataGoTransportError, ItemInfoQuery, KRX_LISTED_ITEM_INFO_ENDPOINT,
    KRX_LISTED_ITEM_INFO_PATH,
};
use domain::{BatchId, TradingDate, UtcTimestamp};
use serde_json::{Map, Value};

use crate::contract::{FetchMode, MARKET_KR, RawEnvelope, RequestMetadata, ResponseKind};
use crate::storage::{BatchSpec, ManifestEntry, RawStore, StoreError};

/// Stable Raw provider scope.
pub const FSC_KRX_LISTED_PROVIDER: &str = "fsc-krx-listed";
/// Exact allowlisted host/path, kept without a query string for metadata.
pub const FSC_KRX_LISTED_ENDPOINT: &str = KRX_LISTED_ITEM_INFO_ENDPOINT;
pub const FSC_KRX_LISTED_PATH: &str = KRX_LISTED_ITEM_INFO_PATH;
/// The separate Raw kind used to keep this identity evidence out of EOD and
/// disclosure publication paths.
pub const FSC_KRX_LISTED_RESPONSE_KIND: ResponseKind = ResponseKind::CandidateMaster;
/// Documented page-size maximum used by the adapter.
pub const ITEM_INFO_PAGE_SIZE: u32 = data_go_client::ITEM_INFO_PAGE_SIZE;
/// Hard bound on one exact-identity pagination walk.
pub const ITEM_INFO_MAX_PAGES: usize = 10;
/// The public source-contract reference recorded on Raw manifests. It is not
/// a credential and does not claim historical retention semantics.
pub const FSC_KRX_LISTED_ENTITLEMENT_REFERENCE: &str =
    "https://www.data.go.kr/data/15094775/openapi.do";

/// One exact listed identity. Both fields are opaque checked-in contract
/// values; the adapter never derives an ISIN from a short code or parses
/// either value. Production storage remains limited to [`FIXED_ETF11`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedEtfIdentity {
    pub short_code: &'static str,
    pub isin_cd: &'static str,
}

/// Fixed launch universe from `configs/universes/kr-etf-core-v1.yaml`.
///
/// The ISIN strings are carried as exact query values. No fuzzy `like*` query
/// and no runtime conversion from `short_code` is permitted.
pub const FIXED_ETF11: [FixedEtfIdentity; 11] = [
    FixedEtfIdentity {
        short_code: "069500",
        isin_cd: "KR7069500007",
    },
    FixedEtfIdentity {
        short_code: "102110",
        isin_cd: "KR7102110004",
    },
    FixedEtfIdentity {
        short_code: "229200",
        isin_cd: "KR7229200001",
    },
    FixedEtfIdentity {
        short_code: "143850",
        isin_cd: "KR7143850006",
    },
    FixedEtfIdentity {
        short_code: "133690",
        isin_cd: "KR7133690008",
    },
    FixedEtfIdentity {
        short_code: "195930",
        isin_cd: "KR7195930003",
    },
    FixedEtfIdentity {
        short_code: "192090",
        isin_cd: "KR7192090009",
    },
    FixedEtfIdentity {
        short_code: "148070",
        isin_cd: "KR7148070006",
    },
    FixedEtfIdentity {
        short_code: "114260",
        isin_cd: "KR7114260003",
    },
    FixedEtfIdentity {
        short_code: "153130",
        isin_cd: "KR7153130000",
    },
    FixedEtfIdentity {
        short_code: "132030",
        isin_cd: "KR7132030008",
    },
];

/// Async read seam. A live implementation is provided by the allowlisted
/// `data-go-client`; tests use fixture readers and never touch the network.
pub trait FscKrxListedRead: std::fmt::Debug + Send + Sync {
    fn get_item_info(
        &self,
        query: &ItemInfoQuery,
    ) -> impl Future<Output = Result<Vec<u8>, FscKrxListedError>> + Send;
}

impl FscKrxListedRead for DataGoClient {
    async fn get_item_info(&self, query: &ItemInfoQuery) -> Result<Vec<u8>, FscKrxListedError> {
        DataGoClient::get_item_info(self, query)
            .await
            .map_err(FscKrxListedError::Transport)
    }
}

/// Closed error taxonomy for the provider and Raw validation boundary.
/// Provider `resultMsg` and body prose never enter this type.
#[derive(Debug, thiserror::Error)]
pub enum FscKrxListedError {
    #[error("FSC KRX listed response was not valid JSON")]
    MalformedJson,
    #[error("FSC KRX listed response did not match the documented shape")]
    UndocumentedShape,
    #[error("FSC KRX listed response body exceeded the permitted bound")]
    ResponseBodyTooLarge { max_bytes: usize },
    #[error("FSC KRX listed response is missing documented field {field}")]
    MissingField { field: &'static str },
    #[error("FSC KRX listed response field {field} has the wrong type")]
    WrongFieldType { field: &'static str },
    #[error("FSC KRX listed provider rejected the request with resultCode {code:?}")]
    NonSuccessResult { code: String },
    #[error("FSC KRX listed response identified page {response}, requested page was {requested}")]
    ResponsePageMismatch { requested: u32, response: u32 },
    #[error("FSC KRX listed response numOfRows differs from the requested page size")]
    ResponsePageSizeMismatch,
    #[error("FSC KRX listed response totalCount changed during one identity walk")]
    InconsistentTotalCount,
    #[error(
        "FSC KRX listed response bytes for {short_code} page {page_no} duplicate {duplicate_short_code} page {duplicate_page_no}"
    )]
    DuplicateResponseBytes {
        short_code: String,
        page_no: u32,
        duplicate_short_code: String,
        duplicate_page_no: u32,
    },
    #[error("FSC KRX listed pagination exceeded {max_pages} pages")]
    PaginationBoundExceeded { max_pages: usize },
    #[error("FSC KRX listed target observation is missing for fixed ETF {short_code}")]
    MissingTargetObservation { short_code: String },
    #[error(
        "FSC KRX listed response contained a non-target observation for fixed ETF {short_code}"
    )]
    UnexpectedObservation { short_code: String },
    #[error(
        "FSC KRX listed response contained duplicate target observations for fixed ETF {short_code}"
    )]
    DuplicateTargetObservation { short_code: String },
    #[error("FSC KRX listed request metadata attempted to include serviceKey")]
    KeyLeakDetected,
    #[error("FSC KRX listed ingest requires a non-empty entitlement reference")]
    MissingEntitlementReference,
    #[error("FSC KRX listed query configuration is invalid")]
    InvalidQuery,
    #[error("FSC KRX listed transport failure: {0}")]
    Transport(DataGoTransportError),
    #[error("FSC KRX listed Raw store failure: {0}")]
    Store(StoreError),
}

/// A successful immutable Raw commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FscKrxListedOutcome {
    Stored(ManifestEntry),
}

/// Result of one non-persisting exact-date availability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FscKrxListedAvailability {
    Available,
    Unavailable,
}

/// Raw-only fixed ETF11 provider.
pub struct FscKrxListedProvider<R: FscKrxListedRead> {
    reader: R,
}

impl<R: FscKrxListedRead> std::fmt::Debug for FscKrxListedProvider<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FscKrxListedProvider")
            .finish_non_exhaustive()
    }
}

impl<R: FscKrxListedRead> FscKrxListedProvider<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Checks one exact date and one fixed identity without constructing Raw
    /// metadata or writing any response bytes. The probe is deliberately
    /// single-page: an exact ISIN/date lookup that claims more than one page
    /// is not accepted as availability evidence.
    pub async fn probe_fixed_identity(
        &self,
        date: &TradingDate,
        identity: FixedEtfIdentity,
    ) -> Result<FscKrxListedAvailability, FscKrxListedError> {
        let bas_dt = api_date(date);
        let query = ItemInfoQuery::new(ITEM_INFO_PAGE_SIZE, 1, bas_dt.clone(), identity.isin_cd)
            .map_err(|_| FscKrxListedError::InvalidQuery)?;
        let bytes = self.reader.get_item_info(&query).await?;
        if bytes.len() > data_go_client::MAX_RESPONSE_BODY_BYTES {
            return Err(FscKrxListedError::ResponseBodyTooLarge {
                max_bytes: data_go_client::MAX_RESPONSE_BODY_BYTES,
            });
        }
        let page = parse_page(&bytes)?;
        if page.page_no != 1 {
            return Err(FscKrxListedError::ResponsePageMismatch {
                requested: 1,
                response: page.page_no,
            });
        }
        if page.num_of_rows != ITEM_INFO_PAGE_SIZE {
            return Err(FscKrxListedError::ResponsePageSizeMismatch);
        }
        if page.total_count == 0 {
            if !page.items.is_empty() {
                return Err(FscKrxListedError::UndocumentedShape);
            }
            return Ok(FscKrxListedAvailability::Unavailable);
        }
        if page.total_count != 1 || page.items.len() != 1 {
            return Err(FscKrxListedError::UndocumentedShape);
        }
        validate_observation(&page.items[0], &bas_dt, identity)?;
        Ok(FscKrxListedAvailability::Available)
    }

    /// Fetches exactly one `basDt` + `isinCd` query for each fixed ETF11
    /// identity, then commits all validated pages as one immutable batch.
    /// Nothing is stored if any identity, page, or response fails validation.
    pub async fn ingest_fixed_etf11(
        &self,
        store: &RawStore,
        date: &TradingDate,
        retrieved_at: UtcTimestamp,
        mode: FetchMode,
        entitlement_reference: &str,
    ) -> Result<FscKrxListedOutcome, FscKrxListedError> {
        if entitlement_reference.trim().is_empty() {
            return Err(FscKrxListedError::MissingEntitlementReference);
        }

        let bas_dt = api_date(date);
        let batch_id = BatchId::generate();
        let mut envelopes = Vec::new();
        let mut seen_response_bytes: Vec<(String, u32, Vec<u8>)> = Vec::new();

        for identity in FIXED_ETF11 {
            let mut expected_total_count = None;
            let mut found_target = false;
            let mut terminated = false;

            for page_no in 1..=(ITEM_INFO_MAX_PAGES as u32) {
                let query = ItemInfoQuery::new(
                    ITEM_INFO_PAGE_SIZE,
                    page_no,
                    bas_dt.clone(),
                    identity.isin_cd,
                )
                .map_err(|_| FscKrxListedError::InvalidQuery)?;
                let bytes = self.reader.get_item_info(&query).await?;
                if bytes.len() > data_go_client::MAX_RESPONSE_BODY_BYTES {
                    return Err(FscKrxListedError::ResponseBodyTooLarge {
                        max_bytes: data_go_client::MAX_RESPONSE_BODY_BYTES,
                    });
                }
                let page = parse_page(&bytes)?;

                if page.page_no != page_no {
                    return Err(FscKrxListedError::ResponsePageMismatch {
                        requested: page_no,
                        response: page.page_no,
                    });
                }
                if page.num_of_rows != ITEM_INFO_PAGE_SIZE {
                    return Err(FscKrxListedError::ResponsePageSizeMismatch);
                }
                if let Some(expected) = expected_total_count {
                    if expected != page.total_count {
                        return Err(FscKrxListedError::InconsistentTotalCount);
                    }
                } else {
                    expected_total_count = Some(page.total_count);
                }

                for (duplicate_short_code, duplicate_page_no, previous) in &seen_response_bytes {
                    if previous == &bytes {
                        return Err(FscKrxListedError::DuplicateResponseBytes {
                            short_code: identity.short_code.to_owned(),
                            page_no,
                            duplicate_short_code: duplicate_short_code.clone(),
                            duplicate_page_no: *duplicate_page_no,
                        });
                    }
                }
                seen_response_bytes.push((identity.short_code.to_owned(), page_no, bytes.clone()));

                for item in page.items {
                    validate_observation(&item, &bas_dt, identity)?;
                    if found_target {
                        return Err(FscKrxListedError::DuplicateTargetObservation {
                            short_code: identity.short_code.to_owned(),
                        });
                    }
                    found_target = true;
                }

                let request = redacted_metadata(&query, mode)?;
                let file_name = format!("item-info-{}-page-{page_no:04}.json", identity.short_code);
                envelopes.push(RawEnvelope::new(
                    batch_id,
                    FSC_KRX_LISTED_RESPONSE_KIND,
                    file_name,
                    bytes,
                    retrieved_at,
                    request,
                ));

                let total_count = page.total_count;
                if total_count == 0
                    || (u64::from(page_no) * u64::from(ITEM_INFO_PAGE_SIZE)) >= total_count
                {
                    terminated = true;
                    break;
                }
            }

            if !terminated {
                return Err(FscKrxListedError::PaginationBoundExceeded {
                    max_pages: ITEM_INFO_MAX_PAGES,
                });
            }
            if !found_target {
                return Err(FscKrxListedError::MissingTargetObservation {
                    short_code: identity.short_code.to_owned(),
                });
            }
        }

        let spec = BatchSpec {
            provider: FSC_KRX_LISTED_PROVIDER,
            market: MARKET_KR,
            date,
            batch_id,
            entitlement_reference: Some(entitlement_reference),
            mode,
        };
        let entry = store
            .store_batch(&spec, &envelopes)
            .map_err(FscKrxListedError::Store)?;
        Ok(FscKrxListedOutcome::Stored(entry))
    }
}

fn api_date(date: &TradingDate) -> String {
    date.to_iso().replace('-', "")
}

fn api_short_code(identity: FixedEtfIdentity) -> String {
    format!("A{}", identity.short_code)
}

fn validate_observation(
    item: &ListedItem,
    bas_dt: &str,
    identity: FixedEtfIdentity,
) -> Result<(), FscKrxListedError> {
    if item.bas_dt != bas_dt
        || item.isin_cd != identity.isin_cd
        || item.srtn_cd != api_short_code(identity)
    {
        return Err(FscKrxListedError::UnexpectedObservation {
            short_code: identity.short_code.to_owned(),
        });
    }
    Ok(())
}

/// The only metadata constructor in this module. Authentication is omitted,
/// not optionally redacted after the fact.
fn redacted_metadata(
    query: &ItemInfoQuery,
    mode: FetchMode,
) -> Result<RequestMetadata, FscKrxListedError> {
    let visible_query = query.visible_pairs();
    if visible_query.iter().any(|(key, _)| key == "serviceKey") {
        return Err(FscKrxListedError::KeyLeakDetected);
    }
    Ok(RequestMetadata {
        endpoint: FSC_KRX_LISTED_ENDPOINT.to_owned(),
        query: visible_query,
        headers: Vec::new(),
        mode,
    })
}

#[derive(Debug)]
struct ListedPage {
    num_of_rows: u32,
    page_no: u32,
    total_count: u64,
    items: Vec<ListedItem>,
}

#[derive(Debug)]
struct ListedItem {
    bas_dt: String,
    srtn_cd: String,
    isin_cd: String,
    #[allow(dead_code)]
    mrkt_ctg: String,
    #[allow(dead_code)]
    itms_nm: String,
    #[allow(dead_code)]
    crno: String,
    #[allow(dead_code)]
    corp_nm: String,
}

fn parse_page(bytes: &[u8]) -> Result<ListedPage, FscKrxListedError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| FscKrxListedError::MalformedJson)?;
    // The official guide documents an XML `<response>` root. In JSON that
    // envelope is preserved as the single top-level `response` property.
    // Require it exactly so a fixture-only unwrapped shape cannot silently
    // diverge from the live provider contract again.
    let document = exact_object(&value, &["response"])?;
    let root = exact_object(required_value(document, "response")?, &["header", "body"])?;
    let header = exact_object(
        required_value(root, "header")?,
        &["resultCode", "resultMsg"],
    )?;
    let result_code = required_string(header, "resultCode")?;
    // Validate the documented field's type, but never retain or print its
    // provider-controlled prose.
    let _ = required_string(header, "resultMsg")?;
    if result_code != "00" {
        return Err(FscKrxListedError::NonSuccessResult {
            code: sanitize_result_code(result_code),
        });
    }

    let body = exact_object(
        required_value(root, "body")?,
        &["numOfRows", "pageNo", "totalCount", "items"],
    )?;
    let num_of_rows = to_u32(required_u64(body, "numOfRows")?)?;
    let page_no = to_u32(required_u64(body, "pageNo")?)?;
    let total_count = required_u64(body, "totalCount")?;
    let items_object = exact_object(required_value(body, "items")?, &["item"])?;
    let items_value = required_value(items_object, "item")?;
    let items = items_value
        .as_array()
        .ok_or(FscKrxListedError::WrongFieldType {
            field: "items.item",
        })?
        .iter()
        .map(parse_item)
        .collect::<Result<Vec<_>, _>>()?;
    if items.len() > num_of_rows as usize {
        return Err(FscKrxListedError::UndocumentedShape);
    }
    Ok(ListedPage {
        num_of_rows,
        page_no,
        total_count,
        items,
    })
}

fn parse_item(value: &Value) -> Result<ListedItem, FscKrxListedError> {
    let object = exact_object(
        value,
        &[
            "basDt", "srtnCd", "isinCd", "mrktCtg", "itmsNm", "crno", "corpNm",
        ],
    )?;
    Ok(ListedItem {
        bas_dt: required_string(object, "basDt")?.to_owned(),
        srtn_cd: required_string(object, "srtnCd")?.to_owned(),
        isin_cd: required_string(object, "isinCd")?.to_owned(),
        mrkt_ctg: required_string(object, "mrktCtg")?.to_owned(),
        itms_nm: required_string(object, "itmsNm")?.to_owned(),
        crno: required_string(object, "crno")?.to_owned(),
        corp_nm: required_string(object, "corpNm")?.to_owned(),
    })
}

fn exact_object<'a>(
    value: &'a Value,
    expected_fields: &[&'static str],
) -> Result<&'a Map<String, Value>, FscKrxListedError> {
    let object = value
        .as_object()
        .ok_or(FscKrxListedError::UndocumentedShape)?;
    for field in expected_fields {
        if !object.contains_key(*field) {
            return Err(FscKrxListedError::MissingField { field });
        }
    }
    if object.len() != expected_fields.len() {
        return Err(FscKrxListedError::UndocumentedShape);
    }
    Ok(object)
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a Value, FscKrxListedError> {
    object
        .get(field)
        .ok_or(FscKrxListedError::MissingField { field })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, FscKrxListedError> {
    required_value(object, field)?
        .as_str()
        .ok_or(FscKrxListedError::WrongFieldType { field })
}

fn required_u64(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<u64, FscKrxListedError> {
    required_value(object, field)?
        .as_u64()
        .ok_or(FscKrxListedError::WrongFieldType { field })
}

fn to_u32(value: u64) -> Result<u32, FscKrxListedError> {
    u32::try_from(value).map_err(|_| FscKrxListedError::UndocumentedShape)
}

fn sanitize_result_code(code: &str) -> String {
    if !code.is_empty() && code.len() <= 8 && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        code.to_owned()
    } else {
        "UNDOCUMENTED_RESULT_CODE".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_universe_has_exactly_eleven_opaque_pairs() {
        assert_eq!(FIXED_ETF11.len(), 11);
        assert!(
            FIXED_ETF11
                .iter()
                .all(|identity| identity.short_code.len() == 6 && identity.isin_cd.len() == 12)
        );
        assert!(
            FIXED_ETF11
                .iter()
                .all(|identity| identity.isin_cd.starts_with("KR7"))
        );
    }

    #[test]
    fn result_message_never_crosses_into_non_success_error() {
        let body = serde_json::json!({
            "response": {
                "header": {"resultCode": "99", "resultMsg": "SENTINEL_PROVIDER_MESSAGE"},
                "body": {"numOfRows": 100, "pageNo": 1, "totalCount": 0, "items": {"item": []}}
            }
        });
        let error = parse_page(body.to_string().as_bytes()).unwrap_err();
        assert!(matches!(
            error,
            FscKrxListedError::NonSuccessResult { ref code } if code == "99"
        ));
        assert!(!error.to_string().contains("SENTINEL_PROVIDER_MESSAGE"));
    }

    #[test]
    fn undocumented_extra_fields_fail_closed() {
        let body = serde_json::json!({
            "response": {
                "header": {"resultCode": "00", "resultMsg": "ok"},
                "body": {"numOfRows": 100, "pageNo": 1, "totalCount": 0, "items": {"item": []}, "extra": 1}
            }
        });
        assert!(matches!(
            parse_page(body.to_string().as_bytes()).unwrap_err(),
            FscKrxListedError::UndocumentedShape
        ));
    }

    #[test]
    fn fixture_only_unwrapped_response_is_rejected() {
        let body = serde_json::json!({
            "header": {"resultCode": "00", "resultMsg": "ok"},
            "body": {"numOfRows": 100, "pageNo": 1, "totalCount": 0, "items": {"item": []}}
        });
        assert!(matches!(
            parse_page(body.to_string().as_bytes()).unwrap_err(),
            FscKrxListedError::MissingField { field: "response" }
        ));
    }
}
