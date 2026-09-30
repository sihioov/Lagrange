//! The deliberately narrow, plaintext KIS `H0STCNT0` market wire contract.
//!
//! This module does not know about account/order TRs and it never repairs a
//! frame.  A complete message is checked before a single parsed record is
//! returned to the transport state machine.

use std::fmt;
use std::time::Instant;

use uuid::Uuid;

pub const TR_ID: &str = "H0STCNT0";
pub const WIRE_VERSION: &str = "kis-h0stcnt0-20260914-v1";
pub const FIELD_COUNT: usize = 47;
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;
pub const MAX_RECORDS: usize = 100;
pub const MAX_FIELD_BYTES: usize = 64;
const MAX_INTEGER_DIGITS: usize = 12;

/// Why a stream quote has no REST-style previous-day base price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasePriceReason {
    NotProvidedByChannel,
}

/// A decimal which has passed the stream's lexical and SQL precision checks.
/// It remains text so this crate never silently rounds broker values.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalText(String);

impl DecimalText {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn parse(raw: &str, positive: bool) -> Result<Self, WireError> {
        if raw.is_empty() || raw.contains(['e', 'E']) || raw == "+" || raw == "-" {
            return Err(WireError::NumericInvalid);
        }
        let bytes = raw.as_bytes();
        let mut start = 0;
        if bytes[0] == b'+' || bytes[0] == b'-' {
            start = 1;
        }
        if start == bytes.len() {
            return Err(WireError::NumericInvalid);
        }
        let mut dot = false;
        let mut integer_digits = 0usize;
        let mut fraction_digits = 0usize;
        for &b in &bytes[start..] {
            match b {
                b'0'..=b'9' if !dot => integer_digits += 1,
                b'0'..=b'9' => fraction_digits += 1,
                b'.' if !dot => dot = true,
                _ => return Err(WireError::NumericInvalid),
            }
        }
        if integer_digits == 0
            || integer_digits > MAX_INTEGER_DIGITS
            || integer_digits + fraction_digits > 20
            || fraction_digits > 8
        {
            return Err(WireError::NumericInvalid);
        }
        let negative = bytes[0] == b'-';
        let is_zero = bytes[start..].iter().all(|b| *b == b'0' || *b == b'.');
        if positive && (negative || is_zero) {
            return Err(WireError::NumericInvalid);
        }
        if raw.starts_with('+') {
            return Err(WireError::NumericInvalid);
        }
        Ok(Self(raw.to_owned()))
    }

    fn is_zero(&self) -> bool {
        let raw = self.0.trim_start_matches('-');
        raw.bytes().all(|b| b == b'0' || b == b'.')
    }

    fn is_positive(&self) -> bool {
        !self.is_zero() && !self.is_negative()
    }

    fn is_negative(&self) -> bool {
        self.0.starts_with('-') && !self.is_zero()
    }
}

impl fmt::Debug for DecimalText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for DecimalText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    LimitUp,
    Up,
    Flat,
    LimitDown,
    Down,
}

impl Direction {
    fn parse(raw: &str) -> Result<Self, WireError> {
        match raw {
            "1" => Ok(Self::LimitUp),
            "2" => Ok(Self::Up),
            "3" => Ok(Self::Flat),
            "4" => Ok(Self::LimitDown),
            "5" => Ok(Self::Down),
            _ => Err(WireError::DirectionInvalid),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    MessageTooLarge,
    InvalidUtf8,
    ControlCharacter,
    FramingInvalid,
    RecordCountInvalid,
    RecordCountExceeded,
    FieldCountMismatch,
    FieldTooLarge,
    SchemaMismatch,
    SymbolInvalid,
    DateInvalid,
    TimeInvalid,
    NumericInvalid,
    DirectionInvalid,
    DirectionContradiction,
    VolumeInvalid,
    HaltFlagInvalid,
    MarketClassUnsupported,
    FutureObservation,
    OutsideSessionWindow,
    StaleEpochObservation,
    UnsupportedMessage,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MessageTooLarge => "market message exceeds local bound",
            Self::InvalidUtf8 => "market message is not valid UTF-8",
            Self::ControlCharacter => "market message contains a control character",
            Self::FramingInvalid => "market message framing is invalid",
            Self::RecordCountInvalid => "market record count is invalid",
            Self::RecordCountExceeded => "market record count exceeds local bound",
            Self::FieldCountMismatch => "market field count does not match record count",
            Self::FieldTooLarge => "market field exceeds local bound",
            Self::SchemaMismatch => "market wire schema is not the adopted version",
            Self::SymbolInvalid => "market symbol is not a six-digit KRX identity",
            Self::DateInvalid => "market business date is invalid",
            Self::TimeInvalid => "market trade time is invalid",
            Self::NumericInvalid => "market numeric field is invalid",
            Self::DirectionInvalid => "market direction code is invalid",
            Self::DirectionContradiction => "market direction contradicts signed change",
            Self::VolumeInvalid => "market volume is invalid",
            Self::HaltFlagInvalid => "market halt flag is invalid",
            Self::MarketClassUnsupported => "market class is unsupported",
            Self::FutureObservation => "market observation is too far in the future",
            Self::OutsideSessionWindow => "market observation is outside the session window",
            Self::StaleEpochObservation => "market observation is stale for this socket epoch",
            Self::UnsupportedMessage => "market message type is unsupported",
        })
    }
}

impl std::error::Error for WireError {}

/// Local context used for the fields whose meaning depends on the approved
/// KST session.  It is intentionally crate-private: callers provide the
/// opaque typed session proof and cannot disable timing checks by constructing
/// an all-default context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseContext {
    session_date: u32,
    session_start_hhmmss: u32,
    session_end_hhmmss: u32,
    /// UTC milliseconds corresponding to KST midnight of `session_date`.
    session_midnight_ms: Option<i64>,
    received_at_ms: Option<i64>,
    socket_open_ms: Option<i64>,
    require_fresh_epoch_trade: bool,
}

/// Non-secret session evidence supplied by the later producer.  It is the
/// only production configuration accepted by the transport for receipt
/// provenance; a caller cannot provide a raw timestamp in its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketStreamSessionProof {
    session_date: u32,
    session_start_hhmmss: u32,
    session_end_hhmmss: u32,
    session_midnight_ms: i64,
    require_fresh_epoch_trade: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionProofError {
    DateInvalid,
    WindowInvalid,
    MidnightInvalid,
}

impl fmt::Display for SessionProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DateInvalid => "market session proof date is invalid",
            Self::WindowInvalid => "market session proof window is invalid",
            Self::MidnightInvalid => "market session proof midnight is invalid",
        })
    }
}

impl std::error::Error for SessionProofError {}

impl MarketStreamSessionProof {
    pub fn new(
        session_date: u32,
        session_start_hhmmss: u32,
        session_end_hhmmss: u32,
        session_midnight_ms: i64,
    ) -> Result<Self, SessionProofError> {
        validate_date(session_date).map_err(|_| SessionProofError::DateInvalid)?;
        if !valid_window_time(session_start_hhmmss, false)
            || !valid_window_time(session_end_hhmmss, true)
            || session_start_hhmmss >= session_end_hhmmss
        {
            return Err(SessionProofError::WindowInvalid);
        }
        if session_midnight_ms != kst_midnight_unix_ms(session_date) {
            return Err(SessionProofError::MidnightInvalid);
        }
        Ok(Self {
            session_date,
            session_start_hhmmss,
            session_end_hhmmss,
            session_midnight_ms,
            require_fresh_epoch_trade: true,
        })
    }

    pub fn session_date(&self) -> u32 {
        self.session_date
    }

    pub fn session_start_hhmmss(&self) -> u32 {
        self.session_start_hhmmss
    }

    pub fn session_end_hhmmss(&self) -> u32 {
        self.session_end_hhmmss
    }

    pub(crate) fn context_at(self, received_at_ms: i64, socket_open_ms: i64) -> ParseContext {
        ParseContext {
            session_date: self.session_date,
            session_start_hhmmss: self.session_start_hhmmss,
            session_end_hhmmss: self.session_end_hhmmss,
            session_midnight_ms: Some(self.session_midnight_ms),
            received_at_ms: Some(received_at_ms),
            socket_open_ms: Some(socket_open_ms),
            require_fresh_epoch_trade: self.require_fresh_epoch_trade,
        }
    }

    pub(crate) fn validate_current_day(self, now_ms: i64) -> Result<(), SessionProofError> {
        if kst_date_from_unix_ms(now_ms) != self.session_date {
            return Err(SessionProofError::DateInvalid);
        }
        Ok(())
    }

    #[cfg(any(feature = "test-support", test))]
    pub(crate) fn synthetic_default() -> Self {
        // The adopted local fixture date is deliberately explicit and is
        // recorded in the fixture manifest.  Synthetic tests do not claim a
        // fresh broker replay, so they exercise current-date/window proof but
        // not the production <=30s first-trade freshness gate.
        Self {
            session_date: 20260921,
            session_start_hhmmss: 0,
            session_end_hhmmss: 240000,
            session_midnight_ms: 1_789_916_400_000,
            require_fresh_epoch_trade: false,
        }
    }
}

impl Default for ParseContext {
    fn default() -> Self {
        Self {
            session_date: 0,
            session_start_hhmmss: 0,
            session_end_hhmmss: 240_000,
            session_midnight_ms: None,
            received_at_ms: None,
            socket_open_ms: None,
            require_fresh_epoch_trade: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketTradeObservation {
    pub symbol: String,
    pub business_date: u32,
    pub trade_time: u32,
    pub price: DecimalText,
    pub change_amount: DecimalText,
    pub change_percent: DecimalText,
    pub direction: Direction,
    pub trade_volume: u64,
    pub cumulative_volume: u64,
    pub halted: bool,
    pub opening_class: String,
    pub hour_class: String,
    pub market_class: String,
    pub transaction_class: String,
    pub base_price: Option<DecimalText>,
    pub base_price_reason: BasePriceReason,
}

impl MarketTradeObservation {
    pub fn is_regular(&self) -> bool {
        self.opening_class == "20"
            && self.hour_class == "0"
            && self.market_class == "2"
            && self.transaction_class.is_empty()
    }

    pub fn provider_trade_at(&self) -> (u32, u32) {
        (self.business_date, self.trade_time)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonRegularMarketObservation {
    pub symbol: String,
    pub business_date: u32,
    pub trade_time: u32,
    pub opening_class: String,
    pub hour_class: String,
    pub market_class: String,
    pub transaction_class: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedMarketRecord {
    Regular(MarketTradeObservation),
    NonRegular(NonRegularMarketObservation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMarketMessage {
    pub record_count: usize,
    pub records: Vec<ParsedMarketRecord>,
}

/// Receipt metadata is intentionally only constructible inside this crate.
/// A REST response or caller timestamp cannot be forged into a market receipt.
#[derive(Clone)]
pub struct MarketReceipt {
    epoch: Uuid,
    receive_ordinal: u64,
    received_at_ms: i64,
    socket_open_ms: i64,
    session_date: u32,
    received_monotonic: Instant,
    observation: MarketTradeObservation,
}

impl fmt::Debug for MarketReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarketReceipt")
            .field("epoch", &self.epoch)
            .field("receive_ordinal", &self.receive_ordinal)
            .field("received_at_ms", &self.received_at_ms)
            .field("socket_open_ms", &self.socket_open_ms)
            .field("session_date", &self.session_date)
            .field("observation", &self.observation)
            .finish()
    }
}

impl MarketReceipt {
    pub fn epoch(&self) -> Uuid {
        self.epoch
    }

    pub fn receive_ordinal(&self) -> u64 {
        self.receive_ordinal
    }

    pub fn received_at_ms(&self) -> i64 {
        self.received_at_ms
    }

    pub fn socket_open_ms(&self) -> i64 {
        self.socket_open_ms
    }

    pub fn session_date(&self) -> u32 {
        self.session_date
    }

    pub fn received_monotonic(&self) -> Instant {
        self.received_monotonic
    }

    pub fn observation(&self) -> &MarketTradeObservation {
        &self.observation
    }

    pub(crate) fn from_transport(
        epoch: Uuid,
        receive_ordinal: u64,
        received_at_ms: i64,
        received_monotonic: Instant,
        socket_open_ms: i64,
        proof: MarketStreamSessionProof,
        acknowledged_subscription: bool,
        observation: MarketTradeObservation,
    ) -> Result<Self, WireError> {
        if !acknowledged_subscription
            || observation.business_date != proof.session_date
            || !observation.is_regular()
        {
            return Err(WireError::StaleEpochObservation);
        }
        Ok(Self {
            epoch,
            receive_ordinal,
            received_at_ms,
            socket_open_ms,
            session_date: proof.session_date,
            received_monotonic,
            observation,
        })
    }
}

/// Parse a complete H0STCNT0 text message.  No record is returned until the
/// complete packed message has passed all structural and consumed-field checks.
pub(crate) fn parse_market_message(
    bytes: &[u8],
    context: ParseContext,
) -> Result<ParsedMarketMessage, WireError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(WireError::MessageTooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| WireError::InvalidUtf8)?;
    if text.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(WireError::ControlCharacter);
    }
    let mut header = text.splitn(4, '|');
    let prefix = header.next().ok_or(WireError::FramingInvalid)?;
    let tr_id = header.next().ok_or(WireError::FramingInvalid)?;
    let count_text = header.next().ok_or(WireError::FramingInvalid)?;
    let payload = header.next().ok_or(WireError::FramingInvalid)?;
    if prefix != "0" || tr_id != TR_ID || payload.contains('|') {
        return Err(WireError::UnsupportedMessage);
    }
    if count_text.len() != 3 || !count_text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WireError::RecordCountInvalid);
    }
    let record_count = count_text
        .parse::<usize>()
        .map_err(|_| WireError::RecordCountInvalid)?;
    if record_count == 0 {
        return Err(WireError::RecordCountInvalid);
    }
    if record_count > MAX_RECORDS {
        return Err(WireError::RecordCountExceeded);
    }
    let fields: Vec<&str> = payload.split('^').collect();
    if fields.len() != record_count * FIELD_COUNT {
        return Err(WireError::FieldCountMismatch);
    }
    for field in &fields {
        if field.len() > MAX_FIELD_BYTES {
            return Err(WireError::FieldTooLarge);
        }
    }

    let mut records = Vec::with_capacity(record_count);
    for chunk in fields.chunks_exact(FIELD_COUNT) {
        records.push(parse_record(chunk, context)?);
    }
    Ok(ParsedMarketMessage {
        record_count,
        records,
    })
}

fn parse_record(fields: &[&str], context: ParseContext) -> Result<ParsedMarketRecord, WireError> {
    let symbol = fields[0];
    if symbol.len() != 6 || !symbol.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WireError::SymbolInvalid);
    }
    let trade_time = parse_time(fields[1])?;
    let price = DecimalText::parse(fields[2], true)?;
    let direction = Direction::parse(fields[3])?;
    let change_amount = DecimalText::parse(fields[4], false)?;
    let change_percent = DecimalText::parse(fields[5], false)?;
    if !direction_matches(direction, &change_amount, &change_percent) {
        return Err(WireError::DirectionContradiction);
    }
    let trade_volume = parse_volume(fields[12])?;
    let cumulative_volume = parse_volume(fields[13])?;
    let business_date = parse_date(fields[33])?;
    let opening_class = fields[34].to_owned();
    let halt = match fields[35] {
        "Y" => true,
        "N" => false,
        _ => return Err(WireError::HaltFlagInvalid),
    };
    let transaction_class = fields[44].to_owned();
    if !transaction_class.is_empty() {
        return Err(WireError::MarketClassUnsupported);
    }
    let hour_class = fields[43].to_owned();
    let market_class = fields[46].to_owned();
    if opening_class.is_empty() || hour_class.is_empty() || market_class.is_empty() {
        return Err(WireError::MarketClassUnsupported);
    }

    validate_context(business_date, trade_time, context)?;

    if opening_class == "20" && hour_class == "0" && market_class == "2" {
        Ok(ParsedMarketRecord::Regular(MarketTradeObservation {
            symbol: symbol.to_owned(),
            business_date,
            trade_time,
            price,
            change_amount,
            change_percent,
            direction,
            trade_volume,
            cumulative_volume,
            halted: halt,
            opening_class,
            hour_class,
            market_class,
            transaction_class,
            base_price: None,
            base_price_reason: BasePriceReason::NotProvidedByChannel,
        }))
    } else {
        Ok(ParsedMarketRecord::NonRegular(
            NonRegularMarketObservation {
                symbol: symbol.to_owned(),
                business_date,
                trade_time,
                opening_class,
                hour_class,
                market_class,
                transaction_class,
            },
        ))
    }
}

fn direction_matches(
    direction: Direction,
    change_amount: &DecimalText,
    change_percent: &DecimalText,
) -> bool {
    match direction {
        Direction::LimitUp | Direction::Up => {
            change_amount.is_positive() && !change_percent.is_negative()
        }
        Direction::Flat => change_amount.is_zero() && change_percent.is_zero(),
        Direction::LimitDown | Direction::Down => {
            change_amount.is_negative() && !change_percent.is_positive()
        }
    }
}

fn parse_volume(raw: &str) -> Result<u64, WireError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WireError::VolumeInvalid);
    }
    let value = raw.parse::<u64>().map_err(|_| WireError::VolumeInvalid)?;
    if value > i64::MAX as u64 {
        return Err(WireError::VolumeInvalid);
    }
    Ok(value)
}

fn parse_date(raw: &str) -> Result<u32, WireError> {
    if raw.len() != 8 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WireError::DateInvalid);
    }
    let year = raw[0..4]
        .parse::<u32>()
        .map_err(|_| WireError::DateInvalid)?;
    let month = raw[4..6]
        .parse::<u32>()
        .map_err(|_| WireError::DateInvalid)?;
    let day = raw[6..8]
        .parse::<u32>()
        .map_err(|_| WireError::DateInvalid)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || days == 0 || day == 0 || day > days {
        return Err(WireError::DateInvalid);
    }
    Ok(year * 10_000 + month * 100 + day)
}

fn validate_date(date: u32) -> Result<(), WireError> {
    let raw = format!("{date:08}");
    parse_date(&raw).map(|_| ())
}

fn valid_window_time(value: u32, allow_midnight_end: bool) -> bool {
    if allow_midnight_end && value == 240000 {
        return true;
    }
    let hour = value / 10_000;
    let minute = (value / 100) % 100;
    let second = value % 100;
    hour <= 23 && minute <= 59 && second <= 59
}

fn kst_date_from_unix_ms(unix_ms: i64) -> u32 {
    let local_days = unix_ms
        .saturating_add(9 * 60 * 60 * 1_000)
        .div_euclid(86_400_000);
    let (year, month, day) = civil_from_days(local_days);
    (year as u32) * 10_000 + (month as u32) * 100 + day as u32
}

fn kst_midnight_unix_ms(date: u32) -> i64 {
    let year = i64::from(date / 10_000);
    let month = i64::from((date / 100) % 100);
    let day = i64::from(date % 100);
    // Days since the Unix epoch, then convert KST midnight to UTC.
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = adjusted_year.div_euclid(400);
    let yoe = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_prime + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86_400_000 - 9 * 60 * 60 * 1_000
}

fn civil_from_days(unix_days: i64) -> (i64, i64, i64) {
    let z = unix_days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn parse_time(raw: &str) -> Result<u32, WireError> {
    if raw.len() != 6 || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WireError::TimeInvalid);
    }
    let hour = raw[0..2]
        .parse::<u32>()
        .map_err(|_| WireError::TimeInvalid)?;
    let minute = raw[2..4]
        .parse::<u32>()
        .map_err(|_| WireError::TimeInvalid)?;
    let second = raw[4..6]
        .parse::<u32>()
        .map_err(|_| WireError::TimeInvalid)?;
    if hour > 23 || minute > 59 || second > 59 {
        return Err(WireError::TimeInvalid);
    }
    Ok(hour * 10_000 + minute * 100 + second)
}

fn hhmmss_seconds(value: u32) -> i64 {
    i64::from(value / 10_000) * 3600 + i64::from((value / 100) % 100) * 60 + i64::from(value % 100)
}

fn validate_context(date: u32, time: u32, context: ParseContext) -> Result<(), WireError> {
    if context.session_date != 0 && date != context.session_date {
        return Err(WireError::DateInvalid);
    }
    if time < context.session_start_hhmmss || time >= context.session_end_hhmmss {
        return Err(WireError::OutsideSessionWindow);
    }
    if context.require_fresh_epoch_trade
        && (context.session_midnight_ms.is_none()
            || context.received_at_ms.is_none()
            || context.socket_open_ms.is_none())
    {
        return Err(WireError::StaleEpochObservation);
    }
    let Some(midnight) = context.session_midnight_ms else {
        return Ok(());
    };
    if midnight != kst_midnight_unix_ms(context.session_date) {
        return Err(WireError::DateInvalid);
    }
    let event_ms = midnight.saturating_add(hhmmss_seconds(time).saturating_mul(1_000));
    if let Some(received) = context.received_at_ms {
        let window_start = midnight
            .saturating_add(hhmmss_seconds(context.session_start_hhmmss).saturating_mul(1_000));
        let window_end = midnight.saturating_add(
            hhmmss_seconds(context.session_end_hhmmss % 240000).saturating_mul(1_000)
                + if context.session_end_hhmmss == 240000 {
                    86_400_000
                } else {
                    0
                },
        );
        if kst_date_from_unix_ms(received) != context.session_date
            || received < window_start
            || received >= window_end
        {
            return Err(WireError::OutsideSessionWindow);
        }
        if event_ms > received.saturating_add(2_000) {
            return Err(WireError::FutureObservation);
        }
        if context.require_fresh_epoch_trade {
            if let Some(opened) = context.socket_open_ms
                && (event_ms < opened || received.saturating_sub(event_ms) > 30_000)
            {
                return Err(WireError::StaleEpochObservation);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<String> {
        let mut f = vec![String::new(); FIELD_COUNT];
        f[0] = "005930".into();
        f[1] = "100001".into();
        f[2] = "70000".into();
        f[3] = "2".into();
        f[4] = "100".into();
        f[5] = "0.14".into();
        f[12] = "10".into();
        f[13] = "20".into();
        f[33] = "20260921".into();
        f[34] = "20".into();
        f[35] = "N".into();
        f[43] = "0".into();
        f[46] = "2".into();
        f
    }

    fn message(f: &[String]) -> String {
        format!("0|{TR_ID}|001|{}", f.join("^"))
    }

    #[test]
    fn current_47_field_message_parses_without_fabricating_base_price() {
        let parsed =
            parse_market_message(&message(&fields()).into_bytes(), ParseContext::default())
                .expect("valid synthetic record");
        let ParsedMarketRecord::Regular(observation) = &parsed.records[0] else {
            panic!("expected regular observation")
        };
        assert_eq!(observation.symbol, "005930");
        assert_eq!(observation.base_price, None);
        assert!(!observation.halted);
        assert_eq!(
            observation.base_price_reason,
            BasePriceReason::NotProvidedByChannel
        );
    }

    #[test]
    fn obsolete_and_overfull_shapes_fail_closed() {
        let mut legacy = fields();
        legacy.pop();
        assert_eq!(
            parse_market_message(
                format!("0|{TR_ID}|001|{}", legacy.join("^")).as_bytes(),
                ParseContext::default()
            ),
            Err(WireError::FieldCountMismatch)
        );
        let mut over = fields();
        over.push("x".into());
        assert_eq!(
            parse_market_message(
                format!("0|{TR_ID}|001|{}", over.join("^")).as_bytes(),
                ParseContext::default()
            ),
            Err(WireError::FieldCountMismatch)
        );
    }

    #[test]
    fn four_packed_records_preserve_empty_auxiliary_fields() {
        let mut all = Vec::new();
        for i in 0..4 {
            let mut f = fields();
            f[12] = (10 + i).to_string();
            all.extend(f);
        }
        let text = format!("0|{TR_ID}|004|{}", all.join("^"));
        let parsed = parse_market_message(text.as_bytes(), ParseContext::default()).unwrap();
        assert_eq!(parsed.record_count, 4);
        assert_eq!(parsed.records.len(), 4);
    }

    #[test]
    fn nonregular_market_class_is_not_a_quote() {
        let mut f = fields();
        f[46] = "3".into();
        let parsed =
            parse_market_message(&message(&f).into_bytes(), ParseContext::default()).unwrap();
        assert!(matches!(
            parsed.records[0],
            ParsedMarketRecord::NonRegular(_)
        ));
    }

    #[test]
    fn undocumented_transaction_class_is_not_interpreted() {
        let mut f = fields();
        f[44] = "1".into();
        assert_eq!(
            parse_market_message(&message(&f).into_bytes(), ParseContext::default()),
            Err(WireError::MarketClassUnsupported)
        );
    }

    #[test]
    fn missing_market_eligibility_code_fails_closed() {
        let mut f = fields();
        f[46].clear();
        assert_eq!(
            parse_market_message(&message(&f).into_bytes(), ParseContext::default()),
            Err(WireError::MarketClassUnsupported)
        );
    }

    #[test]
    fn framing_count_and_size_bounds_fail_closed() {
        assert_eq!(
            parse_market_message(b"0|H0STCNT0|1|", ParseContext::default()),
            Err(WireError::RecordCountInvalid)
        );
        assert_eq!(
            parse_market_message(b"0|H0STCNT0|001|bad|pipe", ParseContext::default()),
            Err(WireError::UnsupportedMessage)
        );
        let mut oversized_field = fields();
        oversized_field[6] = "x".repeat(MAX_FIELD_BYTES + 1);
        assert_eq!(
            parse_market_message(
                &message(&oversized_field).into_bytes(),
                ParseContext::default()
            ),
            Err(WireError::FieldTooLarge)
        );
        assert_eq!(
            parse_market_message(&vec![b'x'; MAX_MESSAGE_BYTES + 1], ParseContext::default()),
            Err(WireError::MessageTooLarge)
        );
        let mut too_large_volume = fields();
        too_large_volume[13] = (i64::MAX as u128 + 1).to_string();
        assert_eq!(
            parse_market_message(
                &message(&too_large_volume).into_bytes(),
                ParseContext::default()
            ),
            Err(WireError::VolumeInvalid)
        );
    }

    #[test]
    fn signed_direction_mismatch_is_rejected() {
        let mut f = fields();
        f[3] = "5".into();
        assert_eq!(
            parse_market_message(&message(&f).into_bytes(), ParseContext::default()),
            Err(WireError::DirectionContradiction)
        );
    }

    #[test]
    fn numeric_precision_and_both_signed_changes_are_checked() {
        let mut too_wide = fields();
        too_wide[2] = "12345678901234567890".into();
        assert_eq!(
            parse_market_message(&message(&too_wide).into_bytes(), ParseContext::default()),
            Err(WireError::NumericInvalid)
        );
        let mut percent_mismatch = fields();
        percent_mismatch[5] = "-0.14".into();
        assert_eq!(
            parse_market_message(
                &message(&percent_mismatch).into_bytes(),
                ParseContext::default()
            ),
            Err(WireError::DirectionContradiction)
        );
        let mut down_percent_mismatch = fields();
        down_percent_mismatch[3] = "5".into();
        down_percent_mismatch[4] = "-100".into();
        down_percent_mismatch[5] = "0.14".into();
        assert_eq!(
            parse_market_message(
                &message(&down_percent_mismatch).into_bytes(),
                ParseContext::default()
            ),
            Err(WireError::DirectionContradiction)
        );
        let mut rounded_zero = fields();
        rounded_zero[5] = "0".into();
        assert!(
            parse_market_message(
                &message(&rounded_zero).into_bytes(),
                ParseContext::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn control_bytes_and_invalid_utf8_fail_before_record_parsing() {
        let mut f = fields();
        f[1] = "235959".into();
        assert!(parse_market_message(&message(&f).into_bytes(), ParseContext::default()).is_ok());
        let mut with_newline = message(&fields()).into_bytes();
        with_newline[0] = b'\n';
        assert_eq!(
            parse_market_message(&with_newline, ParseContext::default()),
            Err(WireError::ControlCharacter)
        );
        assert_eq!(
            parse_market_message(&[0xff], ParseContext::default()),
            Err(WireError::InvalidUtf8)
        );
    }

    #[test]
    fn context_rejects_missing_fresh_epoch_clock_proof() {
        let mut context = ParseContext::default();
        context.require_fresh_epoch_trade = true;
        assert_eq!(
            parse_market_message(&message(&fields()).into_bytes(), context),
            Err(WireError::StaleEpochObservation)
        );
    }

    #[test]
    fn session_proof_rejects_unrelated_midnight() {
        assert_eq!(
            MarketStreamSessionProof::new(20260921, 90000, 153000, 1).unwrap_err(),
            SessionProofError::MidnightInvalid
        );
    }
}
