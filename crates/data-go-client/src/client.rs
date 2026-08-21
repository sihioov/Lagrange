use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::credential::{CredentialSource, SystemCredentialSource};
use crate::error::DataGoTransportError;
use crate::transport::{HttpRequest, LiveTransport, Transport, classify};

/// The documented page-size maximum used by the fixed ETF11 adapter.
pub const ITEM_INFO_PAGE_SIZE: u32 = 100;
const MAX_RETRIES: u32 = 2;
const MAX_RETRY_AFTER_SECS: u64 = 60;

/// Exact visible query for one page of `getItemInfo`.
///
/// The authentication parameter is deliberately not represented here. The
/// private client adds it only after metadata construction is complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemInfoQuery {
    pub num_of_rows: u32,
    pub page_no: u32,
    pub bas_dt: String,
    pub isin_cd: String,
}

impl ItemInfoQuery {
    pub fn new(
        num_of_rows: u32,
        page_no: u32,
        bas_dt: impl Into<String>,
        isin_cd: impl Into<String>,
    ) -> Result<Self, DataGoTransportError> {
        let query = Self {
            num_of_rows,
            page_no,
            bas_dt: bas_dt.into(),
            isin_cd: isin_cd.into(),
        };
        query.validate()?;
        Ok(query)
    }

    /// Revalidates a query at the client boundary. The fields remain visible
    /// for the Raw adapter's fixture seam, so direct struct construction must
    /// not be able to bypass the fixed-universe contract.
    pub fn validate(&self) -> Result<(), DataGoTransportError> {
        if self.num_of_rows == 0 || self.num_of_rows > ITEM_INFO_PAGE_SIZE || self.page_no == 0 {
            return Err(DataGoTransportError::InvalidQuery);
        }
        if !is_valid_yyyymmdd(&self.bas_dt) {
            return Err(DataGoTransportError::InvalidDate);
        }
        if !crate::APPROVED_FIXED_ETF11_ISINS.contains(&self.isin_cd.as_str()) {
            return Err(DataGoTransportError::UnapprovedIsin);
        }
        if self.isin_cd.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(DataGoTransportError::InvalidQuery);
        }
        Ok(())
    }

    /// The only query pairs that may be recorded in Raw metadata or passed
    /// from the adapter to this client. `resultType=json` is fixed because
    /// the portal otherwise defaults to XML while the Raw adapter validates
    /// the documented JSON envelope.
    pub fn visible_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("numOfRows".to_owned(), self.num_of_rows.to_string()),
            ("pageNo".to_owned(), self.page_no.to_string()),
            ("basDt".to_owned(), self.bas_dt.clone()),
            ("isinCd".to_owned(), self.isin_cd.clone()),
            ("resultType".to_owned(), "json".to_owned()),
        ]
    }
}

struct ClientCore<T: Transport> {
    transport: T,
    credentials: Box<dyn CredentialSource + Send + Sync>,
    state: Mutex<RateState>,
    config: ClientConfig,
}

#[derive(Debug, Default)]
struct RateState {
    last_sent: Option<Instant>,
}

impl<T: Transport> ClientCore<T> {
    fn new(
        transport: T,
        credentials: Box<dyn CredentialSource + Send + Sync>,
        config: ClientConfig,
    ) -> Self {
        Self {
            transport,
            credentials,
            state: Mutex::new(RateState::default()),
            config,
        }
    }

    async fn get_item_info(&self, query: &ItemInfoQuery) -> Result<Vec<u8>, DataGoTransportError> {
        query.validate()?;
        let key = self
            .credentials
            .load()
            .map_err(DataGoTransportError::Credential)?;
        let mut visible_query = query.visible_pairs();
        // Construction-time only: this pair exists solely inside the private
        // request object and is never handed to market-data metadata.
        visible_query.push(("serviceKey".to_owned(), key.expose().clone()));
        let request = HttpRequest {
            path: crate::KRX_LISTED_ITEM_INFO_PATH,
            query: visible_query,
        };

        let mut state = self.state.lock().await;
        let mut attempt = 0;
        loop {
            if let Some(last_sent) = state.last_sent {
                let elapsed = last_sent.elapsed();
                if elapsed < self.config.min_request_interval {
                    tokio::time::sleep(self.config.min_request_interval - elapsed).await;
                }
            }

            let outcome = self.transport.send(&request).await;
            state.last_sent = Some(Instant::now());
            match outcome {
                Err(failure) => {
                    if attempt < MAX_RETRIES
                        && matches!(failure, crate::transport::Failure::NeverSent)
                    {
                        attempt += 1;
                        continue;
                    }
                    return Err(classify(failure));
                }
                Ok(response) => {
                    if (300..400).contains(&response.status) {
                        return Err(DataGoTransportError::Redirected {
                            status: response.status,
                        });
                    }
                    if (response.status == 429 || response.status >= 500) && attempt < MAX_RETRIES {
                        if let Some(delay) = response.retry_after_secs {
                            tokio::time::sleep(Duration::from_secs(
                                delay.min(MAX_RETRY_AFTER_SECS),
                            ))
                            .await;
                        }
                        attempt += 1;
                        continue;
                    }
                    if !(200..300).contains(&response.status) {
                        return Err(DataGoTransportError::UnexpectedStatus {
                            status: response.status,
                        });
                    }
                    return Ok(response.body);
                }
            }
        }
    }
}

/// Explicit timeout and pacing configuration. Production uses one request per
/// second; tests may use zero pacing.
#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub min_request_interval: Duration,
}

/// The concrete client whose only public request is the allowlisted listing
/// information operation.
pub struct DataGoClient {
    core: ClientCore<LiveTransport>,
}

impl std::fmt::Debug for DataGoClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataGoClient")
            .field("credentials", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl DataGoClient {
    pub fn new(
        credentials: Box<dyn CredentialSource + Send + Sync>,
        config: ClientConfig,
    ) -> Result<Self, DataGoTransportError> {
        Ok(Self {
            core: ClientCore::new(
                LiveTransport::new(config.connect_timeout, config.read_timeout)?,
                credentials,
                config,
            ),
        })
    }

    pub fn with_default_credentials(config: ClientConfig) -> Result<Self, DataGoTransportError> {
        let credentials = SystemCredentialSource::from_env_or_default()
            .map_err(DataGoTransportError::Credential)?;
        Self::new(Box::new(credentials), config)
    }

    pub async fn get_item_info(
        &self,
        query: &ItemInfoQuery,
    ) -> Result<Vec<u8>, DataGoTransportError> {
        query.validate()?;
        self.core.get_item_info(query).await
    }
}

fn is_valid_yyyymmdd(value: &str) -> bool {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let year = value[0..4].parse::<u32>().ok();
    let month = value[4..6].parse::<u32>().ok();
    let day = value[6..8].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day)) = (year, month, day) else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year % 400 == 0 || (year % 4 == 0 && year % 100 != 0);
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days_in_month).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::{CredentialError, Secret};
    use crate::transport::{Failure, HttpResponse};
    use std::sync::{Arc, Mutex as StdMutex};

    type QueryPairs = Vec<(String, String)>;
    type RecordedRequest = Arc<StdMutex<Option<QueryPairs>>>;

    struct FixedCredential;
    impl CredentialSource for FixedCredential {
        fn load(&self) -> Result<Secret<String>, CredentialError> {
            Ok(Secret::new("service-key-sentinel-test-only".to_owned()))
        }
    }

    struct RecordingTransport {
        request: RecordedRequest,
    }

    impl Transport for RecordingTransport {
        async fn send(&self, request: &HttpRequest) -> Result<HttpResponse, Failure> {
            *self.request.lock().unwrap() = Some(request.query.clone());
            Ok(HttpResponse {
                status: 200,
                body: b"{}".to_vec(),
                retry_after_secs: None,
            })
        }
    }

    fn config() -> ClientConfig {
        ClientConfig {
            connect_timeout: Duration::from_millis(50),
            read_timeout: Duration::from_millis(50),
            min_request_interval: Duration::ZERO,
        }
    }

    #[test]
    fn visible_query_contains_only_exact_documented_non_auth_fields() {
        let query = ItemInfoQuery::new(ITEM_INFO_PAGE_SIZE, 1, "20260820", "KR7069500007")
            .expect("valid query");
        assert_eq!(
            query.visible_pairs(),
            vec![
                ("numOfRows".to_owned(), "100".to_owned()),
                ("pageNo".to_owned(), "1".to_owned()),
                ("basDt".to_owned(), "20260820".to_owned()),
                ("isinCd".to_owned(), "KR7069500007".to_owned()),
                ("resultType".to_owned(), "json".to_owned()),
            ]
        );
        assert!(
            query
                .visible_pairs()
                .iter()
                .all(|(key, _)| key != "serviceKey")
        );
    }

    #[test]
    fn invalid_query_is_typed_and_does_not_carry_values() {
        let error = ItemInfoQuery::new(0, 1, "20260820", "KR7069500007").unwrap_err();
        assert_eq!(error, DataGoTransportError::InvalidQuery);
        assert!(!error.to_string().contains("20260820"));
    }

    #[test]
    fn arbitrary_isin_is_rejected_by_the_low_level_query() {
        let error =
            ItemInfoQuery::new(ITEM_INFO_PAGE_SIZE, 1, "20260820", "KR7999990000").unwrap_err();
        assert_eq!(error, DataGoTransportError::UnapprovedIsin);
    }

    #[test]
    fn date_requires_an_exact_valid_yyyymmdd_calendar_date() {
        for value in ["2026-08-20", "20260230", "2026082", "2026A820"] {
            let error =
                ItemInfoQuery::new(ITEM_INFO_PAGE_SIZE, 1, value, "KR7069500007").unwrap_err();
            assert_eq!(error, DataGoTransportError::InvalidDate);
        }
        assert!(ItemInfoQuery::new(ITEM_INFO_PAGE_SIZE, 1, "20260820", "KR7069500007").is_ok());
    }

    #[test]
    fn direct_public_query_fields_are_revalidated_before_transport() {
        let query = ItemInfoQuery {
            num_of_rows: ITEM_INFO_PAGE_SIZE,
            page_no: 1,
            bas_dt: "20260820".to_owned(),
            isin_cd: "KR7999990000".to_owned(),
        };
        assert_eq!(query.validate(), Err(DataGoTransportError::UnapprovedIsin));
    }

    #[tokio::test]
    async fn service_key_is_added_only_inside_private_transport_request() {
        let request = Arc::new(StdMutex::new(None));
        let core = ClientCore::new(
            RecordingTransport {
                request: request.clone(),
            },
            Box::new(FixedCredential),
            config(),
        );
        let query = ItemInfoQuery::new(100, 1, "20260820", "KR7069500007").unwrap();
        core.get_item_info(&query).await.unwrap();
        let seen = request.lock().unwrap().clone().unwrap();
        assert!(
            seen.iter()
                .any(|(key, value)| key == "resultType" && value == "json")
        );
        assert!(seen.iter().any(|(key, value)| {
            key == "serviceKey" && value == "service-key-sentinel-test-only"
        }));
        assert_eq!(
            seen.iter().filter(|(key, _)| key == "serviceKey").count(),
            1
        );
    }

    #[tokio::test]
    async fn retries_are_bounded_without_printing_provider_details() {
        struct NeverSent {
            calls: Arc<StdMutex<u32>>,
        }
        impl Transport for NeverSent {
            async fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, Failure> {
                *self.calls.lock().unwrap() += 1;
                Err(Failure::NeverSent)
            }
        }
        let calls = Arc::new(StdMutex::new(0));
        let core = ClientCore::new(
            NeverSent {
                calls: calls.clone(),
            },
            Box::new(FixedCredential),
            config(),
        );
        let query = ItemInfoQuery::new(100, 1, "20260820", "KR7069500007").unwrap();
        let error = core.get_item_info(&query).await.unwrap_err();
        assert_eq!(error, DataGoTransportError::NeverSent);
        assert_eq!(*calls.lock().unwrap(), 3);
        assert!(!error.to_string().contains("service-key-sentinel-test-only"));
    }
}
