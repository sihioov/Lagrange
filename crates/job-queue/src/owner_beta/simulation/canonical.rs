//! Strict JSON parsing and deterministic serialization for the simulation
//! result contract.
//!
//! The parser checks object keys before constructing `serde_json::Value`, so
//! a duplicate can never be hidden by the map's last-value behavior. Numeric
//! tokens are restricted to canonical JSON integers; decimal values belong in
//! validated string fields.

use std::str::FromStr;

use serde::Serialize;
use serde_json::{Map, Number, Value};

use super::SimulationContractError;

const MAX_JSON_NESTING: usize = 64;

pub(crate) fn parse_strict_json(bytes: &[u8]) -> Result<Value, SimulationContractError> {
    let mut parser = Parser { bytes, cursor: 0 };
    parser.skip_whitespace();
    let value = parser.parse_value(0)?;
    parser.skip_whitespace();
    if parser.cursor != bytes.len() {
        return Err(SimulationContractError::InvalidJson);
    }
    Ok(value)
}

pub(crate) fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, SimulationContractError> {
    let value = serde_json::to_value(value).map_err(|_| SimulationContractError::InvalidJson)?;
    let mut bytes = Vec::new();
    write_value(&value, &mut bytes)?;
    Ok(bytes)
}

fn write_value(value: &Value, output: &mut Vec<u8>) -> Result<(), SimulationContractError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Number(number) => {
            let text = number.to_string();
            if !is_canonical_integer(&text) {
                return Err(SimulationContractError::InvalidJson);
            }
            output.extend_from_slice(text.as_bytes());
        }
        Value::String(value) => {
            serde_json::to_writer(output, value)
                .map_err(|_| SimulationContractError::InvalidJson)?;
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_value(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if !key.is_ascii() {
                    return Err(SimulationContractError::InvalidJson);
                }
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)
                    .map_err(|_| SimulationContractError::InvalidJson)?;
                output.push(b':');
                write_value(item, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn is_canonical_integer(text: &str) -> bool {
    if text == "0" {
        return true;
    }
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty()
        && digits.as_bytes()[0].is_ascii_digit()
        && digits.as_bytes()[0] != b'0'
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && !(text.starts_with('-') && digits.bytes().all(|byte| byte == b'0'))
}

struct Parser<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl Parser<'_> {
    fn skip_whitespace(&mut self) {
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.cursor += 1;
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<Value, SimulationContractError> {
        self.skip_whitespace();
        match self.bytes.get(self.cursor).copied() {
            Some(b'{') if depth < MAX_JSON_NESTING => self.parse_object(depth),
            Some(b'[') if depth < MAX_JSON_NESTING => self.parse_array(depth),
            Some(b'{' | b'[') => Err(SimulationContractError::InvalidJson),
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b't') => self.parse_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.parse_literal(b"false", Value::Bool(false)),
            Some(b'n') => self.parse_literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_integer(),
            _ => Err(SimulationContractError::InvalidJson),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Value, SimulationContractError> {
        self.cursor += 1;
        self.skip_whitespace();
        let mut object = Map::new();
        if self.consume(b'}') {
            return Ok(Value::Object(object));
        }
        loop {
            self.skip_whitespace();
            if self.bytes.get(self.cursor) != Some(&b'"') {
                return Err(SimulationContractError::InvalidJson);
            }
            let key = self.parse_string()?;
            if object.contains_key(&key) {
                return Err(SimulationContractError::DuplicateJsonKey);
            }
            self.skip_whitespace();
            if !self.consume(b':') {
                return Err(SimulationContractError::InvalidJson);
            }
            let value = self.parse_value(depth + 1)?;
            object.insert(key, value);
            self.skip_whitespace();
            if self.consume(b'}') {
                break;
            }
            if !self.consume(b',') {
                return Err(SimulationContractError::InvalidJson);
            }
        }
        Ok(Value::Object(object))
    }

    fn parse_array(&mut self, depth: usize) -> Result<Value, SimulationContractError> {
        self.cursor += 1;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.consume(b']') {
            return Ok(Value::Array(values));
        }
        loop {
            values.push(self.parse_value(depth + 1)?);
            self.skip_whitespace();
            if self.consume(b']') {
                break;
            }
            if !self.consume(b',') {
                return Err(SimulationContractError::InvalidJson);
            }
        }
        Ok(Value::Array(values))
    }

    fn parse_string(&mut self) -> Result<String, SimulationContractError> {
        let start = self.cursor;
        if !self.consume(b'"') {
            return Err(SimulationContractError::InvalidJson);
        }
        loop {
            match self.bytes.get(self.cursor).copied() {
                Some(b'"') => {
                    self.cursor += 1;
                    break;
                }
                Some(b'\\') => {
                    self.cursor += 1;
                    if self.cursor >= self.bytes.len() {
                        return Err(SimulationContractError::InvalidJson);
                    }
                    self.cursor += 1;
                }
                Some(0x00..=0x1f) | None => return Err(SimulationContractError::InvalidJson),
                Some(_) => self.cursor += 1,
            }
        }
        serde_json::from_slice(&self.bytes[start..self.cursor])
            .map_err(|_| SimulationContractError::InvalidJson)
    }

    fn parse_literal(
        &mut self,
        literal: &[u8],
        value: Value,
    ) -> Result<Value, SimulationContractError> {
        let end = self.cursor + literal.len();
        if self.bytes.get(self.cursor..end) != Some(literal) {
            return Err(SimulationContractError::InvalidJson);
        }
        self.cursor = end;
        Ok(value)
    }

    fn parse_integer(&mut self) -> Result<Value, SimulationContractError> {
        let start = self.cursor;
        if self.consume(b'-') && self.bytes.get(self.cursor).is_none() {
            return Err(SimulationContractError::InvalidJson);
        }
        if self.bytes.get(self.cursor) == Some(&b'0') {
            self.cursor += 1;
            if self.bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                return Err(SimulationContractError::NonCanonicalInteger);
            }
        } else {
            let Some(b'1'..=b'9') = self.bytes.get(self.cursor).copied() else {
                return Err(SimulationContractError::InvalidJson);
            };
            self.cursor += 1;
            while self.bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                self.cursor += 1;
            }
        }
        if matches!(self.bytes.get(self.cursor), Some(b'.' | b'e' | b'E' | b'+')) {
            return Err(SimulationContractError::NonCanonicalInteger);
        }
        let raw = std::str::from_utf8(&self.bytes[start..self.cursor])
            .map_err(|_| SimulationContractError::InvalidJson)?;
        if raw == "-0" {
            return Err(SimulationContractError::NonCanonicalInteger);
        }
        let number = Number::from_str(raw).map_err(|_| SimulationContractError::InvalidJson)?;
        Ok(Value::Number(number))
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.bytes.get(self.cursor) == Some(&byte) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_keys_before_map_conversion() {
        assert_eq!(
            parse_strict_json(br#"{"outer":{"x":1,"x":2}}"#),
            Err(SimulationContractError::DuplicateJsonKey)
        );
        assert_eq!(
            parse_strict_json(br#"{"outer":{"x":1,"\u0078":2}}"#),
            Err(SimulationContractError::DuplicateJsonKey)
        );
    }

    #[test]
    fn rejects_malformed_and_trailing_json() {
        for input in [
            b"{".as_slice(),
            br#"{"x":1} {}"#,
            br#"{"x":\"unterminated}"#,
        ] {
            assert_eq!(
                parse_strict_json(input),
                Err(SimulationContractError::InvalidJson)
            );
        }
    }

    #[test]
    fn rejects_noncanonical_numeric_tokens() {
        for input in [
            br#"{"x":-0}"#.as_slice(),
            br#"{"x":01}"#,
            br#"{"x":1.0}"#,
            br#"{"x":1e0}"#,
        ] {
            assert_eq!(
                parse_strict_json(input),
                Err(SimulationContractError::NonCanonicalInteger)
            );
        }
    }

    #[test]
    fn emits_compact_ascii_key_sorted_json() {
        let value = parse_strict_json(br#" { "z": [true, 2], "a": {"b":1,"A":0} } "#).unwrap();
        let mut bytes = Vec::new();
        write_value(&value, &mut bytes).unwrap();
        assert_eq!(bytes, br#"{"a":{"A":0,"b":1},"z":[true,2]}"#);
    }

    #[test]
    fn accepts_limit_and_rejects_excessive_nesting() {
        let at_limit = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_NESTING),
            "]".repeat(MAX_JSON_NESTING)
        );
        assert!(parse_strict_json(at_limit.as_bytes()).is_ok());

        let over_limit = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_NESTING + 1),
            "]".repeat(MAX_JSON_NESTING + 1)
        );
        assert_eq!(
            parse_strict_json(over_limit.as_bytes()),
            Err(SimulationContractError::InvalidJson)
        );
    }
}
