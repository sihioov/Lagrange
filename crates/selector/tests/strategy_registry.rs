//! Todo 17 red-first integration suite: the strategy registry and promotion
//! gates (design §6.7, FR-STR-001..005).
//!
//! Covered here (Rust side of the registry):
//! - (a) all five baseline packages register and validate (ID / SemVer /
//!   schema / defaults / market / cadence / factors / lookback / risk);
//! - (c) state transitions Draft|Validated|Paper|LiveCandidate|Retired and
//!   the promotion-gate evidence matrix (golden+holdout+cost -> Validated,
//!   parity+observation window -> Paper, Phase 3 safety evidence ->
//!   LiveCandidate, Owner-only Retired);
//! - (d) unauthorized promotion is denied AND audited;
//! - (e) a mutated published version is rejected (immutability);
//! - (f) arbitrary Member code upload is denied (Member changes are
//!   schema-bound configs only);
//! - (g) unsupported market / cadence / asset class -> typed error;
//! - (h) after a new release, old runs still resolve the ORIGINAL immutable
//!   version (typed version resolution).
//!
//! The Python suite (`nt/strategies/tests`) covers (b) the target generators
//! against golden fixtures and the NT adapters over Todo 13 custom events.

use std::collections::BTreeSet;

use domain::StrategyVersion;
use selector::baseline::baseline_packages;
use selector::registry::{
    Actor, AuditOutcome, Cadence, Market, PHASE3_SAFETY_CHECKS, PromotionEvidence, Registry,
    RegistryError, StrategyPackage, StrategyState,
};

const BASELINE_IDS: [&str; 5] = [
    "buy_and_hold",
    "trend_following",
    "relative_momentum",
    "dual_momentum",
    "inverse_volatility",
];

fn owner() -> Actor {
    Actor::Owner
}

fn member(user: &str) -> Actor {
    Actor::Member(user.to_owned())
}

fn golden_evidence() -> PromotionEvidence {
    PromotionEvidence::Golden {
        golden_manifest_hash: "sha256:golden".to_owned(),
        holdout_manifest_hash: "sha256:holdout".to_owned(),
        cost_manifest_hash: "sha256:cost".to_owned(),
    }
}

fn paper_evidence() -> PromotionEvidence {
    PromotionEvidence::Paper {
        parity_report_id: "parity-report-1".to_owned(),
        observation_sessions: 30,
    }
}

fn phase3_evidence() -> PromotionEvidence {
    PromotionEvidence::Phase3 {
        safety_bundle_id: "phase3-bundle-1".to_owned(),
        checks: PHASE3_SAFETY_CHECKS.iter().map(|c| c.to_string()).collect(),
    }
}

/// Registers all five baseline packages (Owner) and returns the registry.
fn registry_with_baselines() -> Registry {
    let mut registry = Registry::new();
    for package in baseline_packages() {
        registry
            .register(&owner(), package)
            .expect("baseline registers");
    }
    registry
}

fn package_by_id<'a>(registry: &'a Registry, id: &str) -> &'a StrategyPackage {
    registry.resolve_latest(id).expect("baseline resolved")
}

#[test]
fn strategy_registry_all_five_baseline_packages_validate() {
    let registry = registry_with_baselines();
    let ids: BTreeSet<&str> = registry
        .all_packages()
        .iter()
        .map(|p| p.strategy_id.as_str())
        .collect();
    assert_eq!(ids, BTreeSet::from(BASELINE_IDS));
    assert_eq!(registry.all_packages().len(), 5);

    for package in registry.all_packages() {
        // ID / SemVer
        assert!(StrategyVersion::parse(&package.version.to_string()).is_ok());
        assert_eq!(package.state, StrategyState::Draft);
        // Schema + defaults
        assert!(package.parameter_schema.get("type").is_some());
        assert!(package.parameter_schema.get("properties").is_some());
        // Supported market / asset class / cadence
        assert_eq!(package.markets, vec![Market::Krx]);
        assert_eq!(
            package.asset_classes,
            vec![selector::registry::AssetClass::Etf]
        );
        assert_eq!(package.cadences, vec![Cadence::Daily]);
        // Factors + lookback + risk
        assert!(!package.risk_description.is_empty());
        assert!(!package.description.is_empty());
        assert!(!package.target_generator_ref.is_empty());
        assert!(!package.nt_adapter_ref.is_empty());
        assert!(!package.golden_fixture_refs.is_empty());
        // Immutable content hash assigned at registration
        assert!(package.canonical_hash.starts_with("sha256:"));
        // Empty factor set implies zero lookback (registry rule).
        if package.required_factors.is_empty() {
            assert_eq!(package.minimum_lookback_sessions, 0);
        }
    }
}

#[test]
fn strategy_registry_baseline_packages_are_market_consistent() {
    // The Rust registry metadata mirrors the Python packages: buy_and_hold
    // requires no factors, the momentum families require their documented
    // factor ids, trend requires the trend factors, inverse volatility the
    // realized-vol factor.
    let registry = registry_with_baselines();
    assert!(
        package_by_id(&registry, "buy_and_hold")
            .required_factors
            .is_empty()
    );
    assert_eq!(
        package_by_id(&registry, "buy_and_hold").minimum_lookback_sessions,
        0
    );
    assert_eq!(
        package_by_id(&registry, "trend_following").required_factors,
        BTreeSet::from(["trend_50".to_owned(), "trend_200".to_owned()])
    );
    assert_eq!(
        package_by_id(&registry, "relative_momentum").required_factors,
        BTreeSet::from(["momentum_12_1".to_owned()])
    );
    assert_eq!(
        package_by_id(&registry, "dual_momentum").required_factors,
        BTreeSet::from(["return_12m".to_owned()])
    );
    assert_eq!(
        package_by_id(&registry, "inverse_volatility").required_factors,
        BTreeSet::from(["vol_60".to_owned()])
    );
}

#[test]
fn strategy_registry_register_requires_owner() {
    let mut registry = Registry::new();
    let err = registry.register(&member("alice"), baseline_packages()[0].clone());
    let err = err.expect_err("member registration must be denied");
    assert_eq!(err.code(), "UNAUTHORIZED");
    let denied = registry
        .audit()
        .iter()
        .find(|e| e.outcome == AuditOutcome::Denied && e.action == "REGISTER")
        .expect("audited denial");
    assert_eq!(denied.actor, "member:alice");
}

#[test]
fn strategy_registry_mutated_published_version_rejected() {
    let mut registry = registry_with_baselines();
    let original = package_by_id(&registry, "buy_and_hold").clone();

    // A mutation attempt re-registers the SAME id+version with changed
    // content: the version is immutable and must be rejected.
    let mut mutated = original.clone();
    mutated.risk_description = "mutated risk".to_owned();
    let err = registry
        .register(&owner(), mutated)
        .expect_err("mutated published version must be rejected");
    assert_eq!(err.code(), "IMMUTABLE_VERSION");

    // The registry still resolves the ORIGINAL definition, byte-identical.
    let resolved = package_by_id(&registry, "buy_and_hold");
    assert_eq!(*resolved, original);
    assert_eq!(resolved.canonical_hash, original.canonical_hash);
}

#[test]
fn strategy_registry_old_runs_resolve_original_version_after_new_release() {
    let mut registry = registry_with_baselines();
    let original_hash = package_by_id(&registry, "buy_and_hold")
        .canonical_hash
        .clone();
    let original_risk = package_by_id(&registry, "buy_and_hold")
        .risk_description
        .clone();

    // A new release of the same strategy (new SemVer, changed definition).
    let mut next = package_by_id(&registry, "buy_and_hold").clone();
    next.version = StrategyVersion::parse("1.1.0").expect("semver");
    next.risk_description = "v1.1 risk model update".to_owned();
    registry
        .register(&owner(), next)
        .expect("new release registers");

    assert_eq!(
        registry
            .resolve_latest("buy_and_hold")
            .expect("latest")
            .version
            .to_string(),
        "1.1.0"
    );

    // Old runs still resolve the ORIGINAL immutable version: identical
    // content hash, identical definition, untouched by the release.
    let old = registry
        .resolve("buy_and_hold", "1.0.0")
        .expect("old version resolves");
    assert_eq!(old.canonical_hash, original_hash);
    assert_eq!(old.risk_description, original_risk);
    assert_eq!(old.version.to_string(), "1.0.0");
}

#[test]
fn strategy_registry_unsupported_market_cadence_asset_class_rejected() {
    let mut registry = registry_with_baselines();
    // (g) typed denials at the boundary: unsupported market / cadence.
    let market_err = Market::parse("us").expect_err("us market unsupported");
    assert_eq!(market_err.code(), "UNSUPPORTED_MARKET");
    let cadence_err = Cadence::parse("intraday").expect_err("intraday unsupported");
    assert_eq!(cadence_err.code(), "UNSUPPORTED_CADENCE");

    // A package declaring no market / cadence is an invalid definition.
    let mut no_market = package_by_id(&registry, "buy_and_hold").clone();
    no_market.strategy_id = "no_market_strategy".to_owned();
    no_market.markets = vec![];
    let err = registry
        .register(&owner(), no_market)
        .expect_err("empty markets rejected");
    assert_eq!(err.code(), "INVALID_PACKAGE");

    let mut no_cadence = package_by_id(&registry, "buy_and_hold").clone();
    no_cadence.strategy_id = "no_cadence_strategy".to_owned();
    no_cadence.cadences = vec![];
    let err = registry
        .register(&owner(), no_cadence)
        .expect_err("empty cadences rejected");
    assert_eq!(err.code(), "INVALID_PACKAGE");
}

#[test]
fn strategy_registry_invalid_package_definition_rejected() {
    let mut registry = registry_with_baselines();
    let mut empty_id = package_by_id(&registry, "buy_and_hold").clone();
    empty_id.strategy_id = "".to_owned();
    assert_eq!(
        registry
            .register(&owner(), empty_id)
            .expect_err("empty id")
            .code(),
        "INVALID_PACKAGE"
    );

    let mut bad_id = package_by_id(&registry, "buy_and_hold").clone();
    bad_id.strategy_id = "Buy And Hold!".to_owned();
    assert_eq!(
        registry
            .register(&owner(), bad_id)
            .expect_err("bad id")
            .code(),
        "INVALID_PACKAGE"
    );

    // Defaults must validate against the package's own JSON Schema.
    let mut bad_defaults = package_by_id(&registry, "buy_and_hold").clone();
    bad_defaults.strategy_id = "bad_defaults".to_owned();
    bad_defaults.default_parameters = serde_json::json!({
        "benchmark_instrument": "069500.KRX",
        "target_weight": 2.0,
    });
    assert_eq!(
        registry
            .register(&owner(), bad_defaults)
            .expect_err("bad defaults")
            .code(),
        "INVALID_PACKAGE"
    );

    // Non-Draft registration is rejected (packages enter the registry in
    // Draft only; every other state is reached through promotion gates).
    let mut pre_validated = package_by_id(&registry, "buy_and_hold").clone();
    pre_validated.strategy_id = "pre_validated".to_owned();
    pre_validated.state = StrategyState::Validated;
    assert_eq!(
        registry
            .register(&owner(), pre_validated)
            .expect_err("non-draft registration")
            .code(),
        "INVALID_PACKAGE"
    );

    // Empty factor set with a nonzero lookback is an inconsistent definition.
    let mut inconsistent = package_by_id(&registry, "buy_and_hold").clone();
    inconsistent.strategy_id = "inconsistent".to_owned();
    inconsistent.required_factors = BTreeSet::new();
    inconsistent.minimum_lookback_sessions = 100;
    assert_eq!(
        registry
            .register(&owner(), inconsistent)
            .expect_err("inconsistent lookback")
            .code(),
        "INVALID_PACKAGE"
    );
}

#[test]
fn strategy_registry_validated_gate_requires_golden_holdout_cost() {
    let mut registry = registry_with_baselines();

    // Happy path: golden + holdout + cost checks -> Validated.
    let record = registry
        .promote(
            &owner(),
            "buy_and_hold",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect("golden gate passes");
    assert_eq!(record.from, StrategyState::Draft);
    assert_eq!(record.to, StrategyState::Validated);
    assert_eq!(
        package_by_id(&registry, "buy_and_hold").state,
        StrategyState::Validated
    );

    // Missing evidence -> typed denial naming the missing check.
    let mut incomplete = registry
        .promote(
            &owner(),
            "trend_following",
            "1.0.0",
            StrategyState::Validated,
            PromotionEvidence::Golden {
                golden_manifest_hash: "sha256:golden".to_owned(),
                holdout_manifest_hash: String::new(),
                cost_manifest_hash: "sha256:cost".to_owned(),
            },
        )
        .expect_err("holdout missing");
    assert_eq!(incomplete.code(), "MISSING_PROMOTION_EVIDENCE");

    // Wrong evidence type for the gate -> typed denial.
    incomplete = registry
        .promote(
            &owner(),
            "trend_following",
            "1.0.0",
            StrategyState::Validated,
            paper_evidence(),
        )
        .expect_err("paper evidence is not golden evidence");
    assert_eq!(incomplete.code(), "MISSING_PROMOTION_EVIDENCE");
    assert_eq!(
        package_by_id(&registry, "trend_following").state,
        StrategyState::Draft,
        "denied promotion must not change state"
    );
}

#[test]
fn strategy_registry_paper_gate_requires_parity_and_observation_window() {
    let mut registry = registry_with_baselines();
    registry
        .promote(
            &owner(),
            "trend_following",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect("validated");

    // Happy path: parity + a >= 21-session observation window -> Paper.
    registry
        .promote(
            &owner(),
            "trend_following",
            "1.0.0",
            StrategyState::Paper,
            PromotionEvidence::Paper {
                parity_report_id: "parity-1".to_owned(),
                observation_sessions: 21,
            },
        )
        .expect("minimum window passes");
    assert_eq!(
        package_by_id(&registry, "trend_following").state,
        StrategyState::Paper
    );

    // Observation window below the minimum -> typed denial.
    let mut registry = registry_with_baselines();
    registry
        .promote(
            &owner(),
            "trend_following",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect("validated");
    let err = registry
        .promote(
            &owner(),
            "trend_following",
            "1.0.0",
            StrategyState::Paper,
            PromotionEvidence::Paper {
                parity_report_id: "parity-1".to_owned(),
                observation_sessions: 5,
            },
        )
        .expect_err("window too short");
    assert_eq!(err.code(), "INVALID_PROMOTION");

    // Skipping the Validated gate: Draft -> Paper directly is denied.
    let err = registry
        .promote(
            &owner(),
            "relative_momentum",
            "1.0.0",
            StrategyState::Paper,
            paper_evidence(),
        )
        .expect_err("must pass through Validated");
    assert_eq!(err.code(), "INVALID_PROMOTION");
}

#[test]
fn strategy_registry_live_candidate_gate_requires_phase3_safety_evidence() {
    let mut registry = registry_with_baselines();
    registry
        .promote(
            &owner(),
            "inverse_volatility",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect("validated");
    registry
        .promote(
            &owner(),
            "inverse_volatility",
            "1.0.0",
            StrategyState::Paper,
            paper_evidence(),
        )
        .expect("paper");

    // Happy path: full Phase 3 safety bundle -> LiveCandidate.
    registry
        .promote(
            &owner(),
            "inverse_volatility",
            "1.0.0",
            StrategyState::LiveCandidate,
            phase3_evidence(),
        )
        .expect("phase 3 evidence passes");
    assert_eq!(
        package_by_id(&registry, "inverse_volatility").state,
        StrategyState::LiveCandidate
    );

    // A bundle missing one documented check -> typed denial naming it.
    let mut registry = registry_with_baselines();
    registry
        .promote(
            &owner(),
            "inverse_volatility",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect("validated");
    registry
        .promote(
            &owner(),
            "inverse_volatility",
            "1.0.0",
            StrategyState::Paper,
            paper_evidence(),
        )
        .expect("paper");
    let mut checks: BTreeSet<String> = PHASE3_SAFETY_CHECKS.iter().map(|c| c.to_string()).collect();
    checks.remove("kill_switch");
    let err = registry
        .promote(
            &owner(),
            "inverse_volatility",
            "1.0.0",
            StrategyState::LiveCandidate,
            PromotionEvidence::Phase3 {
                safety_bundle_id: "incomplete-bundle".to_owned(),
                checks,
            },
        )
        .expect_err("kill-switch evidence missing");
    assert_eq!(err.code(), "MISSING_PROMOTION_EVIDENCE");

    // Skipping Paper: Validated -> LiveCandidate directly is denied.
    let mut registry = registry_with_baselines();
    registry
        .promote(
            &owner(),
            "relative_momentum",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect("validated");
    let err = registry
        .promote(
            &owner(),
            "relative_momentum",
            "1.0.0",
            StrategyState::LiveCandidate,
            phase3_evidence(),
        )
        .expect_err("must pass through Paper");
    assert_eq!(err.code(), "INVALID_PROMOTION");
}

#[test]
fn strategy_registry_retired_is_owner_only_and_terminal() {
    let mut registry = registry_with_baselines();

    // Owner retires a Draft package (no evidence required).
    registry
        .retire(&owner(), "buy_and_hold", "1.0.0")
        .expect("owner retires");
    assert_eq!(
        package_by_id(&registry, "buy_and_hold").state,
        StrategyState::Retired
    );

    // Retired is terminal: no promotion out of it.
    let err = registry
        .promote(
            &owner(),
            "buy_and_hold",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect_err("retired is terminal");
    assert_eq!(err.code(), "INVALID_PROMOTION");

    // Members cannot retire (or promote) anything.
    let err = registry
        .retire(&member("alice"), "trend_following", "1.0.0")
        .expect_err("member cannot retire");
    assert_eq!(err.code(), "UNAUTHORIZED");
}

#[test]
fn strategy_registry_unauthorized_promotion_denied_and_audited() {
    let mut registry = registry_with_baselines();

    // (d) A Member attempts promotion with otherwise-valid evidence.
    let err = registry
        .promote(
            &member("alice"),
            "dual_momentum",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect_err("member promotion denied");
    assert_eq!(err.code(), "UNAUTHORIZED");

    // Typed denial is AUDITED: actor, action, outcome, reason, strategy.
    let entry = registry
        .audit()
        .iter()
        .rev()
        .find(|e| e.action == "PROMOTE" && e.outcome == AuditOutcome::Denied)
        .expect("promotion denial audited");
    assert_eq!(entry.actor, "member:alice");
    assert_eq!(entry.strategy_id.as_deref(), Some("dual_momentum"));
    assert_eq!(entry.to_state, Some(StrategyState::Validated));
    assert!(entry.reason.contains("Owner"));

    // State unchanged by the denial.
    assert_eq!(
        package_by_id(&registry, "dual_momentum").state,
        StrategyState::Draft
    );
}

#[test]
fn strategy_registry_member_config_is_schema_bound() {
    let mut registry = registry_with_baselines();

    // A Member may change ONLY validated parameters of an existing version.
    let config = registry
        .apply_member_config(
            &member("alice"),
            "buy_and_hold",
            "1.0.0",
            serde_json::json!({
                "benchmark_instrument": "069500.KRX",
                "target_weight": 0.8,
                "rebalance_cadence": "monthly",
            }),
        )
        .expect("schema-valid member config");
    assert_eq!(config.strategy_version.to_string(), "1.0.0");
    assert_eq!(config.parameters["target_weight"], 0.8);
    assert_eq!(registry.configs().len(), 1);

    // Out-of-range parameter -> typed denial (schema bound).
    let err = registry
        .apply_member_config(
            &member("alice"),
            "buy_and_hold",
            "1.0.0",
            serde_json::json!({ "benchmark_instrument": "069500.KRX", "target_weight": 1.5 }),
        )
        .expect_err("out-of-range rejected");
    assert_eq!(err.code(), "INVALID_PARAMETERS");

    // Unknown property (additionalProperties: false) -> typed denial.
    let err = registry
        .apply_member_config(
            &member("alice"),
            "buy_and_hold",
            "1.0.0",
            serde_json::json!({
                "benchmark_instrument": "069500.KRX",
                "target_weight": 0.5,
                "leverage": 3,
            }),
        )
        .expect_err("unknown property rejected");
    assert_eq!(err.code(), "INVALID_PARAMETERS");

    // The member config never mutates the immutable package.
    let package = package_by_id(&registry, "buy_and_hold");
    assert_eq!(package.state, StrategyState::Draft);
    assert_eq!(package.default_parameters["target_weight"], 1.0);
}

#[test]
fn strategy_registry_member_code_upload_denied() {
    let mut registry = registry_with_baselines();

    // (f) Arbitrary Member code upload is denied at the typed boundary.
    let err = registry
        .deploy_code(
            &member("alice"),
            "def evil(): import os; os.system('rm -rf /')",
        )
        .expect_err("member code denied");
    assert_eq!(err.code(), "MEMBER_CODE_DENIED");

    let denied = registry
        .audit()
        .iter()
        .rev()
        .find(|e| e.action == "DEPLOY_CODE" && e.outcome == AuditOutcome::Denied)
        .expect("code denial audited");
    assert_eq!(denied.actor, "member:alice");

    // Owner deploys strategy code through the deployment boundary.
    registry
        .deploy_code(&owner(), "def run(ctx): return ctx.targets")
        .expect("owner deploys");
    assert_eq!(registry.deployments().len(), 1);
}

#[test]
fn strategy_registry_version_resolution_is_typed() {
    let registry = registry_with_baselines();
    let err = registry
        .resolve("no_such_strategy", "1.0.0")
        .expect_err("unknown id");
    assert_eq!(err.code(), "UNKNOWN_STRATEGY");
    let err = registry
        .resolve("buy_and_hold", "9.9.9")
        .expect_err("unknown version");
    assert_eq!(err.code(), "UNKNOWN_VERSION");
}

#[test]
fn strategy_registry_audit_is_append_only_and_ordered() {
    let mut registry = registry_with_baselines();
    // Exercise several audited operations (successes and denials).
    registry
        .promote(
            &owner(),
            "buy_and_hold",
            "1.0.0",
            StrategyState::Validated,
            golden_evidence(),
        )
        .expect("ok");
    registry
        .promote(
            &member("alice"),
            "buy_and_hold",
            "1.0.0",
            StrategyState::Paper,
            paper_evidence(),
        )
        .expect_err("denied");
    registry
        .deploy_code(&member("bob"), "code")
        .expect_err("denied");
    registry
        .apply_member_config(
            &member("bob"),
            "buy_and_hold",
            "1.0.0",
            serde_json::json!({ "benchmark_instrument": "069500.KRX", "target_weight": 0.4 }),
        )
        .expect("ok");

    let audit = registry.audit();
    // 5 registrations + 1 approved promotion + 1 denied promotion + 1 denied
    // deploy + 1 approved member config = 9 audited operations.
    assert_eq!(audit.len(), 9, "every operation is audited");
    let mut prev_seq = 0;
    for entry in audit {
        assert!(entry.seq > prev_seq, "strictly increasing sequence");
        prev_seq = entry.seq;
        assert!(!entry.actor.is_empty() && !entry.action.is_empty());
        assert!(!entry.reason.is_empty());
    }
    // Approved and denied outcomes coexist in the same ordered log.
    assert!(audit.iter().any(|e| e.outcome == AuditOutcome::Approved));
    assert!(audit.iter().any(|e| e.outcome == AuditOutcome::Denied));
}

#[test]
fn strategy_registry_error_codes_are_stable_wire_values() {
    // Every typed denial exposes a stable machine-readable code.
    assert_eq!(
        RegistryError::UnknownStrategy {
            strategy_id: "x".into()
        }
        .code(),
        "UNKNOWN_STRATEGY"
    );
    assert_eq!(
        RegistryError::UnknownVersion {
            strategy_id: "x".into(),
            version: "1.0.0".into()
        }
        .code(),
        "UNKNOWN_VERSION"
    );
    assert_eq!(
        RegistryError::ImmutableVersion {
            strategy_id: "x".into(),
            version: "1.0.0".into()
        }
        .code(),
        "IMMUTABLE_VERSION"
    );
    assert_eq!(
        RegistryError::Unauthorized {
            actor: "member:alice".into(),
            action: "PROMOTE".into()
        }
        .code(),
        "UNAUTHORIZED"
    );
    assert_eq!(
        RegistryError::MissingPromotionEvidence {
            to_state: "Validated".into(),
            missing: "holdout".into()
        }
        .code(),
        "MISSING_PROMOTION_EVIDENCE"
    );
    assert_eq!(
        RegistryError::InvalidPromotion {
            from: StrategyState::Draft,
            to: StrategyState::Paper,
            detail: "skip".into(),
        }
        .code(),
        "INVALID_PROMOTION"
    );
    assert_eq!(
        RegistryError::MemberCodeDenied { detail: "x".into() }.code(),
        "MEMBER_CODE_DENIED"
    );
    assert_eq!(
        RegistryError::InvalidParameters { detail: "x".into() }.code(),
        "INVALID_PARAMETERS"
    );
}
