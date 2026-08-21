//! Private HTTP boundary. Formatted reqwest errors never leave this module:
//! they can include the complete URL, which would include `serviceKey`.

use std::time::Duration;

use crate::error::DataGoTransportError;

pub(crate) struct HttpRequest {
    pub path: &'static str,
    pub query: Vec<(String, String)>,
}

pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub retry_after_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    NeverSent,
    TimedOut,
    Indeterminate,
    UnreadableBody,
    ResponseTooLarge,
}

pub(crate) fn classify(failure: Failure) -> DataGoTransportError {
    match failure {
        Failure::NeverSent => DataGoTransportError::NeverSent,
        Failure::TimedOut => DataGoTransportError::TimedOut,
        Failure::Indeterminate => DataGoTransportError::Indeterminate,
        Failure::UnreadableBody => DataGoTransportError::UnreadableBody,
        Failure::ResponseTooLarge => DataGoTransportError::ResponseTooLarge,
    }
}

pub(crate) fn classify_send_error(is_connect: bool, is_timeout: bool) -> Failure {
    if is_connect {
        Failure::NeverSent
    } else if is_timeout {
        Failure::TimedOut
    } else {
        Failure::Indeterminate
    }
}

pub(crate) trait Transport: Send + Sync {
    fn send(
        &self,
        request: &HttpRequest,
    ) -> impl std::future::Future<Output = Result<HttpResponse, Failure>> + Send;
}

pub(crate) struct LiveTransport {
    client: reqwest::Client,
}

impl LiveTransport {
    pub fn new(
        connect_timeout: Duration,
        read_timeout: Duration,
    ) -> Result<Self, DataGoTransportError> {
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(read_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| DataGoTransportError::ClientBuildFailed)?;
        Ok(Self { client })
    }
}

impl Transport for LiveTransport {
    async fn send(&self, request: &HttpRequest) -> Result<HttpResponse, Failure> {
        let url = format!("{}{}", crate::DATA_GO_BASE_URL, request.path);
        let mut response = match self.client.get(&url).query(&request.query).send().await {
            Ok(response) => response,
            Err(error) => return Err(classify_send_error(error.is_connect(), error.is_timeout())),
        };
        let status = response.status().as_u16();
        let retry_after_secs = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value <= 60);
        if response
            .content_length()
            .is_some_and(|length| length > crate::MAX_RESPONSE_BODY_BYTES as u64)
        {
            return Err(Failure::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Failure::UnreadableBody)?
        {
            append_bounded_body(&mut body, &chunk)?;
        }
        Ok(HttpResponse {
            status,
            body,
            retry_after_secs,
        })
    }
}

fn append_bounded_body(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), Failure> {
    if chunk.len() > crate::MAX_RESPONSE_BODY_BYTES.saturating_sub(body.len()) {
        return Err(Failure::ResponseTooLarge);
    }
    body.extend_from_slice(chunk);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_classification_discards_library_details() {
        assert_eq!(
            classify(Failure::NeverSent),
            DataGoTransportError::NeverSent
        );
        assert_eq!(classify(Failure::TimedOut), DataGoTransportError::TimedOut);
        assert_eq!(
            classify(Failure::Indeterminate),
            DataGoTransportError::Indeterminate
        );
        assert_eq!(
            classify(Failure::UnreadableBody),
            DataGoTransportError::UnreadableBody
        );
        assert_eq!(
            classify(Failure::ResponseTooLarge),
            DataGoTransportError::ResponseTooLarge
        );
        assert_eq!(classify_send_error(true, true), Failure::NeverSent);
        assert_eq!(classify_send_error(false, true), Failure::TimedOut);
    }

    #[test]
    fn response_body_bound_accepts_exact_limit_and_rejects_one_more_byte() {
        let mut body = Vec::new();
        append_bounded_body(&mut body, &vec![b'x'; crate::MAX_RESPONSE_BODY_BYTES]).unwrap();
        assert_eq!(body.len(), crate::MAX_RESPONSE_BODY_BYTES);
        assert_eq!(
            append_bounded_body(&mut body, b"x"),
            Err(Failure::ResponseTooLarge)
        );
    }

    #[test]
    fn live_client_constructs_offline_with_redirects_disabled() {
        assert!(LiveTransport::new(Duration::from_millis(50), Duration::from_millis(50)).is_ok());
    }
}
