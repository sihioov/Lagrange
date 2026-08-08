//! Instrument and order mapping (plan Todo 36, design §6.12).
//!
//! Every value crossing the broker boundary is translated explicitly. Nothing
//! is passed through on the assumption that our representation happens to
//! match KIS's, because where they differ they differ silently: a KRX ticker
//! is six digits with leading zeros that a numeric round-trip destroys, and an
//! order side is a distinct TR id rather than a field.
//!
//! Parsing a reply is equally explicit. A missing or renamed field is
//! [`KisError::SchemaDrift`], never a defaulted value — defaulting a missing
//! broker order number to an empty string would produce an order the system
//! believes it placed and can never find again.

use crate::error::KisError;

/// KRX instrument id (`069500.KRX`) ⇄ KIS six-digit code (`069500`).
///
/// A struct rather than two free functions so the KRX suffix and the width
/// rule live in one place; the mapping is total for KRX and rejects anything
/// else rather than guessing.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstrumentMapper;

impl InstrumentMapper {
    /// `069500.KRX` → `069500`.
    pub fn to_broker(&self, instrument_id: &str) -> Result<String, KisError> {
        let unknown = || KisError::UnknownInstrument {
            instrument: instrument_id.to_string(),
        };
        let (symbol, venue) = instrument_id.split_once('.').ok_or_else(unknown)?;
        if venue != "KRX" {
            // Phase 3 is KRX-only. Silently accepting another venue would send
            // a code the broker cannot resolve.
            return Err(unknown());
        }
        if symbol.len() != 6 || !symbol.chars().all(|c| c.is_ascii_digit()) {
            return Err(unknown());
        }
        Ok(symbol.to_string())
    }

    /// `069500` → `069500.KRX`.
    pub fn from_broker(&self, code: &str) -> Result<String, KisError> {
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(KisError::UnknownInstrument {
                instrument: code.to_string(),
            });
        }
        Ok(format!("{code}.KRX"))
    }
}

/// Buy or sell, as the broker distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    /// KIS routes side by TR id, not by a body field, so the mapping has to
    /// produce a transaction id rather than a value.
    pub fn tr_id(self) -> &'static str {
        match self {
            // Cash order, real trading.
            Self::Buy => "TTTC0802U",
            Self::Sell => "TTTC0801U",
        }
    }
}

/// How the order should be priced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Limit,
    Market,
}

impl OrderType {
    fn code(self) -> &'static str {
        match self {
            Self::Limit => "00",
            Self::Market => "01",
        }
    }
}

/// An order as this system expresses it, before broker translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRequest {
    pub client_order_id: String,
    pub instrument_id: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    /// Whole shares. KRX cash equities do not trade fractionally, so this is
    /// an integer and a fractional quantity is a bug to surface, not round.
    pub quantity: u64,
    /// Limit price as an exact decimal string; `None` for a market order.
    pub price: Option<String>,
}

/// Translate an order into the KIS `order-cash` body.
///
/// `account` and `product_code` are supplied separately and never stored on
/// the request, so an order struct sitting in a log or a queue carries no
/// account number.
pub fn order_to_broker_body(
    mapper: &InstrumentMapper,
    order: &OrderRequest,
    account: &crate::secret::AccountNo,
    product_code: &str,
) -> Result<String, KisError> {
    let code = mapper.to_broker(&order.instrument_id)?;
    // A market order carries price "0"; sending a limit price with a market
    // order type is a rejection, and sending none with a limit order is worse
    // - KIS treats an absent price as market.
    let price = match (order.order_type, order.price.as_deref()) {
        (OrderType::Market, _) => "0".to_string(),
        (OrderType::Limit, Some(p)) => p.to_string(),
        (OrderType::Limit, None) => {
            return Err(KisError::SchemaDrift {
                endpoint: "order-cash".to_string(),
                detail: "a limit order requires a price; KIS reads an absent price as market"
                    .to_string(),
            });
        }
    };
    Ok(format!(
        r#"{{"CANO":"{cano}","ACNT_PRDT_CD":"{prdt}","PDNO":"{code}","ORD_DVSN":"{dvsn}","ORD_QTY":"{qty}","ORD_UNPR":"{price}"}}"#,
        cano = account.expose(),
        prdt = product_code,
        dvsn = order.order_type.code(),
        qty = order.quantity,
    ))
}

/// What the broker said about a submitted order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderAck {
    pub broker_order_no: String,
    pub exchange_org_no: String,
    pub accepted_at: String,
}

/// Parse a KIS order acknowledgement.
///
/// Every field is required. A missing one is schema drift rather than a
/// default: an empty broker order number would mean an order the system
/// believes it placed and can never look up again.
pub fn parse_order_ack(body: &str) -> Result<OrderAck, KisError> {
    let drift = |detail: &str| KisError::SchemaDrift {
        endpoint: "order-cash".to_string(),
        detail: detail.to_string(),
    };
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| drift(&format!("body is not JSON: {e}")))?;

    // KIS signals application-level failure in rt_cd even on HTTP 200, so a
    // 200 alone is not success.
    match value.get("rt_cd").and_then(|v| v.as_str()) {
        Some("0") => {}
        Some(code) => {
            let msg = value
                .get("msg1")
                .and_then(|v| v.as_str())
                .unwrap_or("no message");
            return Err(KisError::Broker {
                status: 200,
                endpoint: "order-cash".to_string(),
                body: crate::error::redact_payload(&format!("rt_cd={code} msg={msg}")),
            });
        }
        None => return Err(drift("rt_cd is missing")),
    }

    let output = value
        .get("output")
        .ok_or_else(|| drift("output is missing"))?;
    let field = |name: &str| -> Result<String, KisError> {
        output
            .get(name)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| drift(&format!("output.{name} is missing or empty")))
    };

    Ok(OrderAck {
        broker_order_no: field("ODNO")?,
        exchange_org_no: field("KRX_FWDG_ORD_ORGNO")?,
        accepted_at: field("ORD_TMD")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::AccountNo;

    #[test]
    fn krx_instruments_round_trip_preserving_leading_zeros() {
        let m = InstrumentMapper;
        assert_eq!(m.to_broker("069500.KRX").unwrap(), "069500");
        assert_eq!(m.from_broker("069500").unwrap(), "069500.KRX");
        // The leading zero is the whole point: a numeric round-trip loses it
        // and silently addresses a different instrument.
        assert_eq!(m.to_broker("005930.KRX").unwrap(), "005930");
        assert_eq!(m.from_broker("005930").unwrap(), "005930.KRX");
    }

    #[test]
    fn a_non_krx_or_malformed_instrument_is_rejected_not_guessed() {
        let m = InstrumentMapper;
        for bad in [
            "AAPL.NASDAQ",
            "069500",
            "69500.KRX",
            "0695001.KRX",
            "abcdef.KRX",
        ] {
            assert!(
                m.to_broker(bad).is_err(),
                "{bad} should not map to a broker code"
            );
        }
        assert!(m.from_broker("69500").is_err());
        assert!(m.from_broker("06950A").is_err());
    }

    #[test]
    fn side_maps_to_a_transaction_id_not_a_body_field() {
        // KIS routes buy and sell to different TR ids; a "side" field would be
        // silently ignored.
        assert_eq!(OrderSide::Buy.tr_id(), "TTTC0802U");
        assert_eq!(OrderSide::Sell.tr_id(), "TTTC0801U");
        assert_ne!(OrderSide::Buy.tr_id(), OrderSide::Sell.tr_id());
    }

    fn limit_order() -> OrderRequest {
        OrderRequest {
            client_order_id: "coid-1".to_string(),
            instrument_id: "069500.KRX".to_string(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            quantity: 10,
            price: Some("40200".to_string()),
        }
    }

    #[test]
    fn a_limit_order_maps_to_the_kis_body() {
        let body = order_to_broker_body(
            &InstrumentMapper,
            &limit_order(),
            &AccountNo::new("50123456"),
            "01",
        )
        .expect("maps");
        assert!(body.contains(r#""PDNO":"069500""#), "{body}");
        assert!(body.contains(r#""ORD_DVSN":"00""#), "{body}");
        assert!(body.contains(r#""ORD_QTY":"10""#), "{body}");
        assert!(body.contains(r#""ORD_UNPR":"40200""#), "{body}");
    }

    #[test]
    fn a_market_order_sends_price_zero() {
        let mut o = limit_order();
        o.order_type = OrderType::Market;
        o.price = Some("40200".to_string());
        let body =
            order_to_broker_body(&InstrumentMapper, &o, &AccountNo::new("50123456"), "01").unwrap();
        assert!(body.contains(r#""ORD_DVSN":"01""#), "{body}");
        assert!(
            body.contains(r#""ORD_UNPR":"0""#),
            "a market order must not carry a limit price: {body}"
        );
    }

    #[test]
    fn a_limit_order_without_a_price_is_refused() {
        // KIS reads an absent price as MARKET, so letting this through would
        // silently convert a limit order into a market order.
        let mut o = limit_order();
        o.price = None;
        let err = order_to_broker_body(&InstrumentMapper, &o, &AccountNo::new("5"), "01")
            .expect_err("must refuse");
        assert!(err.to_string().contains("market"), "{err}");
    }

    #[test]
    fn an_order_request_carries_no_account_number() {
        // The account is supplied at mapping time, so an OrderRequest sitting
        // in a log or a queue discloses nothing.
        let rendered = format!("{:?}", limit_order());
        assert!(!rendered.contains("50123456"), "{rendered}");
    }

    #[test]
    fn a_successful_ack_is_parsed() {
        let ack = parse_order_ack(&crate::simulator::BrokerSimulator::order_ack("0000117057"))
            .expect("parses");
        assert_eq!(ack.broker_order_no, "0000117057");
        assert_eq!(ack.exchange_org_no, "00950");
        assert_eq!(ack.accepted_at, "090512");
    }

    #[test]
    fn a_missing_order_number_is_schema_drift_not_a_default() {
        // An empty broker order number means an order we believe we placed and
        // can never look up again.
        let body = r#"{"rt_cd":"0","output":{"KRX_FWDG_ORD_ORGNO":"00950","ORD_TMD":"090512"}}"#;
        let err = parse_order_ack(body).expect_err("must be drift");
        assert!(matches!(err, KisError::SchemaDrift { .. }));
        assert!(err.to_string().contains("ODNO"), "{err}");
    }

    #[test]
    fn an_empty_order_number_is_also_schema_drift() {
        let body =
            r#"{"rt_cd":"0","output":{"ODNO":"","KRX_FWDG_ORD_ORGNO":"00950","ORD_TMD":"1"}}"#;
        assert!(matches!(
            parse_order_ack(body),
            Err(KisError::SchemaDrift { .. })
        ));
    }

    #[test]
    fn an_application_level_failure_on_http_200_is_not_success() {
        // KIS signals rejection in rt_cd while still answering 200.
        let body = r#"{"rt_cd":"1","msg1":"주문가능금액이 부족합니다","output":{}}"#;
        let err = parse_order_ack(body).expect_err("rt_cd != 0");
        assert!(matches!(err, KisError::Broker { status: 200, .. }));
        assert!(
            !err.is_ambiguous(),
            "an explicit rejection is not ambiguous"
        );
    }

    #[test]
    fn a_non_json_body_is_schema_drift() {
        let err = parse_order_ack("<html>gateway error</html>").expect_err("not JSON");
        assert!(matches!(err, KisError::SchemaDrift { .. }));
        // And it is never retried, for either kind.
        assert!(!err.is_retryable(crate::error::RequestKind::Read));
        assert!(!err.is_retryable(crate::error::RequestKind::Mutation));
    }
}
