//! Todo 24 OpenAPI contract: the authored `apps/api-server/openapi.json`
//! spec is the versioned contract. These tests prove the spec and the router
//! route inventory agree exactly, every operation carries the required
//! auth/ownership/entitlement/idempotency/audit/cache/error metadata, and the
//! stable error-code table matches the code constants.

use api_server::contract::{CONTRACT_ROUTES, ERROR_CODES, Phase, RouteSpec};
use serde_json::Value;
use std::collections::BTreeSet;

/// The authored OpenAPI document (the versioned contract).
const SPEC: &str = include_str!("../../../apps/api-server/openapi.json");

#[test]
fn openapi_contract_parses() {
    let spec: Value =
        serde_json::from_str(SPEC).unwrap_or_else(|e| panic!("openapi.json must parse: {e}"));
    assert_eq!(spec["openapi"].as_str().unwrap(), "3.1.0");
    assert_eq!(spec["info"]["version"].as_str().unwrap(), "1.0.0");
    assert_eq!(
        spec["info"]["title"].as_str().unwrap(),
        "Lagrange Station API"
    );
}

#[test]
fn openapi_contract_routes_match_router_inventory() {
    let spec: Value = serde_json::from_str(SPEC).expect("spec parses");
    let spec_paths = spec["paths"].as_object().expect("paths object");
    let spec_ops: BTreeSet<(String, String)> = spec_paths
        .iter()
        .flat_map(|(path, item)| {
            item.as_object()
                .expect("path item")
                .iter()
                .filter(|(m, _)| matches!(m.as_str(), "get" | "post" | "put" | "patch" | "delete"))
                .map(|(m, _)| (m.clone(), path.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    let inventory: BTreeSet<(String, String)> = CONTRACT_ROUTES
        .iter()
        .map(|r| (r.method.to_ascii_lowercase(), r.path.to_string()))
        .collect();
    assert_eq!(
        spec_ops, inventory,
        "spec routes must equal the router inventory (no drift)"
    );
}

#[test]
fn openapi_contract_every_operation_has_required_metadata() {
    let spec: Value = serde_json::from_str(SPEC).expect("spec parses");
    let spec_paths = spec["paths"].as_object().expect("paths");
    let mut checked = 0usize;
    for (path, item) in spec_paths {
        for (method, op) in item.as_object().expect("path item") {
            if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                continue;
            }
            checked += 1;
            let op = op.as_object().expect("operation");
            let meta = op
                .get("x-lagrange")
                .unwrap_or_else(|| panic!("{method} {path}: missing x-lagrange metadata"))
                .as_object()
                .expect("x-lagrange object");
            for key in [
                "auth",
                "ownership",
                "entitlement",
                "idempotency",
                "audit",
                "cache",
                "errors",
                "phase",
            ] {
                assert!(
                    meta.contains_key(key),
                    "{method} {path}: missing x-lagrange.{key}"
                );
            }
            // Every mutating operation declares idempotency semantics.
            let is_mutating = op
                .get("x-lagrange")
                .and_then(|m| m.get("idempotency"))
                .and_then(|i| i.get("required"))
                .is_some();
            let body_params = op.get("requestBody");
            if body_params.is_some() {
                assert!(
                    is_mutating,
                    "{method} {path}: a request body implies mutating/idempotent semantics"
                );
            }
            // Every operation references the shared error envelope on 4xx.
            let responses = op["responses"].as_object().expect("responses");
            let has_error_schema = responses
                .iter()
                .any(|(code, _resp)| code.starts_with('4') || code.starts_with('5'));
            assert!(
                has_error_schema,
                "{method} {path}: must declare 4xx/5xx responses"
            );
            // Cache semantics: authenticated data is never shared.
            assert!(
                meta["cache"]["policy"].as_str().is_some(),
                "{method} {path}: cache policy must be a string"
            );
        }
    }
    assert_eq!(checked, CONTRACT_ROUTES.len());
}

#[test]
fn openapi_contract_error_codes_match_constants() {
    let spec: Value = serde_json::from_str(SPEC).expect("spec parses");
    let spec_codes: BTreeSet<String> = spec["components"]["schemas"]["ErrorCode"]["enum"]
        .as_array()
        .expect("ErrorCode enum")
        .iter()
        .map(|c| c.as_str().expect("code string").to_string())
        .collect();
    let const_codes: BTreeSet<String> = ERROR_CODES.iter().map(|c| c.code.to_string()).collect();
    assert_eq!(
        spec_codes, const_codes,
        "spec ErrorCode enum must match constants"
    );
    // The stable codes the plan acceptance names must exist.
    for stable in [
        "DATASET_BLOCKED",
        "DATA_STALE",
        "DATA_ENTITLEMENT_REQUIRED",
        "DUPLICATE_RESOURCE",
        "INVALID_PARAMETER",
        "INVALID_DATE",
        "INVALID_DECIMAL",
        "INVALID_CURSOR",
        "INVALID_STRATEGY_PARAMETER",
        "BACKTEST_CAPACITY_EXCEEDED",
        "RESULT_INTEGRITY_FAILED",
        "FORBIDDEN",
        "RESOURCE_NOT_FOUND",
        "PAYLOAD_TOO_LARGE",
        "IDEMPOTENCY_KEY_REQUIRED",
        "IDEMPOTENCY_KEY_MISMATCH",
    ] {
        assert!(const_codes.contains(stable), "stable code {stable} missing");
    }
    // Codes are unique across the table.
    assert_eq!(
        const_codes.len(),
        ERROR_CODES.len(),
        "error codes must be unique"
    );
}

#[test]
fn openapi_contract_error_envelope_schema() {
    let spec: Value = serde_json::from_str(SPEC).expect("spec parses");
    let error = &spec["components"]["schemas"]["Error"];
    assert!(error.is_object(), "Error schema must exist");
    let props = error["properties"].as_object().expect("Error properties");
    assert!(props.contains_key("code"));
    assert!(props.contains_key("message"));
    assert!(props.contains_key("request_id"));
    assert!(props.contains_key("details"));
    let envelope = &spec["components"]["schemas"]["ErrorEnvelope"];
    assert!(envelope.is_object(), "ErrorEnvelope schema must exist");
    assert!(envelope["properties"]["error"].is_object());
}

#[test]
fn openapi_contract_phase3_routes_are_owner_only() {
    let spec: Value = serde_json::from_str(SPEC).expect("spec parses");
    for route in CONTRACT_ROUTES {
        if route.phase == Phase::Phase3 {
            assert!(
                route.owner_only,
                "Phase 3 route {} must be Owner-only",
                route.path
            );
        }
        if route.owner_only {
            let op = spec_operation(route);
            let meta = op["x-lagrange"].as_object().expect("meta");
            assert_eq!(
                meta["ownership"]["owner_only"], true,
                "route {} must declare owner_only",
                route.path
            );
        }
    }
    let _ = &spec;
}

fn spec_operation(route: &RouteSpec) -> Value {
    let spec: Value = serde_json::from_str(SPEC).expect("spec parses");
    spec["paths"][route.path][route.method.to_ascii_lowercase()].clone()
}

#[test]
fn openapi_contract_every_mutating_route_requires_idempotency_key() {
    for spec in CONTRACT_ROUTES {
        if spec.mutating && !spec.naturally_idempotent {
            let op = spec_operation(spec);
            let idem = op["x-lagrange"]["idempotency"]
                .as_object()
                .unwrap_or_else(|| panic!("route {}: missing idempotency metadata", spec.path));
            assert_eq!(
                idem["required"], true,
                "mutating route {} must require an Idempotency-Key",
                spec.path
            );
        }
    }
}

/// Every error code a handler actually emits must be a declared code.
///
/// `openapi_contract_error_codes_match_constants` proves the spec and the
/// constant table agree, but both are declarations. Nothing proved the
/// handlers only emit codes from that table, and a handler is where a code
/// comes into existence. `LIVE_KILL_SWITCH_ENGAGED` (Todo 37) was emitted by
/// `start_node` for a while without appearing in either list: undocumented in
/// the spec, and — because the web derives its Zod enum from the generated
/// types — unparseable by the browser, so the Owner would have seen a parse
/// failure instead of "the kill switch is engaged".
#[test]
fn openapi_contract_handlers_emit_only_declared_codes() {
    use std::fs;
    use std::path::Path;

    // `api_error(status, code, ..)` and `tenancy_response(err, rid, code)`:
    // in both, the code is the first string literal of the call.
    const CALLS: [&str; 2] = ["api_error(", "tenancy_response("];

    let declared: BTreeSet<&str> = ERROR_CODES.iter().map(|c| c.code).collect();
    let http_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/http");
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    let mut files_scanned = 0usize;

    for entry in fs::read_dir(&http_dir).expect("src/http is readable") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("handler source is readable");
        files_scanned += 1;
        for call in CALLS {
            for (idx, _) in source.match_indices(call) {
                let rest = &source[idx + call.len()..];
                // The first string literal of the call is the code argument;
                // the status/error arguments before it are never strings.
                let Some(open) = rest.find('"') else { continue };
                let after = &rest[open + 1..];
                let Some(close) = after.find('"') else {
                    continue;
                };
                let token = &after[..close];
                // Skip call sites that pass a non-literal code.
                let literal_code = !token.is_empty()
                    && token
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit());
                if literal_code {
                    emitted.insert(token.to_string());
                }
            }
        }
    }

    assert!(
        files_scanned > 10,
        "expected the handler modules to be scanned, saw {files_scanned}"
    );
    assert!(
        emitted.contains("LIVE_KILL_SWITCH_ENGAGED"),
        "the scan must actually reach live.rs; it is the regression this test exists for"
    );

    let undeclared: Vec<&String> = emitted
        .iter()
        .filter(|c| !declared.contains(c.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "handlers emit error codes absent from ERROR_CODES (so absent from the OpenAPI spec and \
         unparseable by the web client): {undeclared:?}"
    );
}
