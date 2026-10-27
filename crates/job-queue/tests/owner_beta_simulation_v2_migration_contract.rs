use std::collections::BTreeMap;

use job_queue::owner_beta::{
    OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA, OwnerBetaStrategySnapshot,
};
use serde_json::{Value, json};

const UP: &str = include_str!("../../../migrations/0064_owner_beta_simulation_v2.up.sql");
const DOWN: &str = include_str!("../../../migrations/0064_owner_beta_simulation_v2.down.sql");
const ORIGINAL_UP: &str =
    include_str!("../../../migrations/0063_owner_beta_price_only_simulations.up.sql");
const ORIGINAL_DOWN: &str =
    include_str!("../../../migrations/0063_owner_beta_price_only_simulations.down.sql");

struct DefaultSnapshot {
    strategy_id: &'static str,
    config_json: &'static str,
    config_sha256: &'static str,
    hash_preimage: &'static str,
}

const V2_DEFAULTS: [DefaultSnapshot; 5] = [
    DefaultSnapshot {
        strategy_id: "buy_and_hold",
        config_json: r#"{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0}"#,
        config_sha256: "sha256:4de76616ae1d72646bd695d0e5aa7df7d6544c2eef5bef7ca949536c1801fc06",
        hash_preimage: r#"{"config_json":{"benchmark_instrument":"069500.KRX","rebalance_cadence":"none","target_weight":1.0},"schema":"owner-beta-strategy-config-v1","strategy_id":"buy_and_hold","strategy_version":"1.0.0"}"#,
    },
    DefaultSnapshot {
        strategy_id: "trend_following",
        config_json: r#"{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200}"#,
        config_sha256: "sha256:90e5b4c6972678e9186d4655d76496676e7cfb8dbe473a249f062963873c04a0",
        hash_preimage: r#"{"config_json":{"benchmark_instrument":"069500.KRX","fast_ma":50,"slow_ma":200},"schema":"owner-beta-strategy-config-v1","strategy_id":"trend_following","strategy_version":"1.0.0"}"#,
    },
    DefaultSnapshot {
        strategy_id: "relative_momentum",
        config_json: r#"{"lookback_months":12,"top_n":3}"#,
        config_sha256: "sha256:edace2510fb491084b142615a709f8ef689a5b207d7bae25ce7a886afceb8d07",
        hash_preimage: r#"{"config_json":{"lookback_months":12,"top_n":3},"schema":"owner-beta-strategy-config-v1","strategy_id":"relative_momentum","strategy_version":"1.0.0"}"#,
    },
    DefaultSnapshot {
        strategy_id: "dual_momentum",
        config_json: r#"{"absolute_threshold":0.0,"lookback_months":12}"#,
        config_sha256: "sha256:ac6a1ac393e02952fe2db6308e5274436f766479f3e631ebe42e429a677426a6",
        hash_preimage: r#"{"config_json":{"absolute_threshold":0.0,"lookback_months":12},"schema":"owner-beta-strategy-config-v1","strategy_id":"dual_momentum","strategy_version":"1.0.0"}"#,
    },
    DefaultSnapshot {
        strategy_id: "inverse_volatility",
        config_json: r#"{"max_weight":0.3,"vol_window":60}"#,
        config_sha256: "sha256:6252f27903f5ea0a7755ea7fc6c05a935518a2dbe44edf371a1e71f127d1aa38",
        hash_preimage: r#"{"config_json":{"max_weight":0.3,"vol_window":60},"schema":"owner-beta-strategy-config-v1","strategy_id":"inverse_volatility","strategy_version":"1.0.0"}"#,
    },
];

fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("FUNCTION public.{name}(");
    let function = source.find(&marker).expect("function definition exists");
    let function = source[..function]
        .rfind("CREATE ")
        .expect("function CREATE starts before its signature");
    let after = &source[function..];
    let as_start = after.find("AS $").expect("dollar-quoted function body") + "AS ".len();
    let tag_end = after[as_start + 1..]
        .find('$')
        .map(|offset| as_start + 1 + offset)
        .expect("dollar quote closes its tag");
    let tag = &after[as_start..=tag_end];
    let body_start = function + tag_end + 1 + 1;
    let terminator = format!("\n{tag};");
    let body_end = source[body_start..]
        .find(&terminator)
        .map(|offset| body_start + offset)
        .expect("dollar-quoted body terminator");
    source[body_start..body_end].trim()
}

fn function_declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("FUNCTION public.{name}(");
    let signature = source.find(&marker).expect("function definition exists");
    let start = source[..signature]
        .rfind("CREATE ")
        .expect("function CREATE starts before its signature");
    let after = &source[start..];
    let end = after
        .find(&format!("ALTER FUNCTION public.{name}"))
        .expect("function ownership statement follows definition");
    &after[..end]
}

#[test]
fn v2_allowlist_matches_real_strategy_snapshots_and_frozen_preimages() {
    assert!(UP.contains("sha256:20dabf650deaff751d8d34a7fea98773e6843b879ed20e6002475fc70c243ee3"));
    assert!(UP.contains("owner-beta-price-only-simulation-job-v2"));
    assert!(UP.contains("ELSE false"));

    for default in &V2_DEFAULTS {
        let config: Value = serde_json::from_str(default.config_json).expect("default JSON");
        let snapshot =
            OwnerBetaStrategySnapshot::from_resolved_config(&job_queue::resolver::ResolvedConfig {
                strategy_id: default.strategy_id.to_owned(),
                strategy_version: "1.0.0".to_owned(),
                config: config.clone(),
            })
            .expect("existing strategy snapshot constructor accepts the frozen default");
        assert_eq!(snapshot.config_json(), &config);
        assert_eq!(snapshot.config_sha256().as_str(), default.config_sha256);
        assert!(UP.contains(default.strategy_id));
        assert!(UP.contains(default.config_json));
        assert!(UP.contains(default.config_sha256));

        let mut preimage = BTreeMap::new();
        preimage.insert("config_json", config);
        preimage.insert(
            "schema",
            Value::String(OWNER_BETA_STRATEGY_CONFIG_SNAPSHOT_SCHEMA.to_owned()),
        );
        preimage.insert("strategy_id", json!(default.strategy_id));
        preimage.insert("strategy_version", json!("1.0.0"));
        assert_eq!(
            serde_json::to_string(&preimage).expect("canonical tagged preimage"),
            default.hash_preimage
        );
    }

    for exact_decimal in [
        "(p_config_json ->> 'target_weight') IS NOT DISTINCT FROM '1.0'",
        "(p_config_json ->> 'absolute_threshold') IS NOT DISTINCT FROM '0.0'",
        "(p_config_json ->> 'max_weight') IS NOT DISTINCT FROM '0.3'",
    ] {
        assert!(UP.contains(exact_decimal), "{exact_decimal}");
    }
}

#[test]
fn payload_and_namespace_bind_schema_contract_strategy_config_and_prefix() {
    assert!(UP.contains("owner_beta_simulation_version_binding_matches("));
    assert!(UP.contains("owner_beta_simulation_version_namespace_matches("));
    assert!(UP.contains("owner-beta:price-only-simulation:v1:"));
    assert!(UP.contains("owner-beta:price-only-simulation:v2:"));
    assert!(UP.contains("pg_catalog.left(p_idempotency_key, 36)"));
    assert!(UP.contains("owner-beta-price-only-simulation-job-v1"));
    assert!(UP.contains("owner-beta-price-only-simulation-job-v2"));
    assert!(UP.contains("NEW.max_attempts <> 3"));
    assert!(UP.contains("jsonb_object_keys(NEW.payload_json)"));
    assert!(UP.contains("jsonb_typeof(NEW.payload_json -> 'pins')"));
    assert!(ORIGINAL_UP.contains("owner_beta_historical_price_only_v1"));

    let old_payload = function_body(ORIGINAL_UP, "owner_beta_simulation_payload_matches");
    let restored_payload = function_body(DOWN, "owner_beta_simulation_payload_matches");
    assert_eq!(restored_payload, old_payload);

    let old_insert = function_body(ORIGINAL_UP, "owner_beta_simulation_jobs_insert_guard");
    let restored_insert = function_body(DOWN, "owner_beta_simulation_jobs_insert_guard");
    assert_eq!(restored_insert, old_insert);

    let old_run = function_body(ORIGINAL_UP, "owner_beta_simulation_runs_validate_binding");
    let restored_run = function_body(DOWN, "owner_beta_simulation_runs_validate_binding");
    assert_eq!(restored_run, old_run);

    let versioned_run = function_body(UP, "owner_beta_simulation_runs_validate_binding");
    for preserved in [
        "current_setting('app.actor_user_id', true)",
        "set_config('app.actor_user_id'",
        "COALESCE(v_actor_before, '')",
        "FOR UPDATE OF job",
        "config.is_active",
        "owner beta simulation run identity is immutable",
    ] {
        assert!(versioned_run.contains(preserved), "{preserved}");
    }
    assert!(versioned_run.contains("owner_beta_simulation_version_namespace_matches"));

    for helper in [
        "owner_beta_simulation_version_binding_matches",
        "owner_beta_simulation_version_namespace_matches",
    ] {
        let declaration = function_declaration(UP, helper);
        assert!(declaration.contains("LANGUAGE sql"), "{helper} remains SQL");
        assert!(declaration.contains("IMMUTABLE"), "{helper} remains pure");
        assert!(
            declaration.contains("SET search_path = pg_catalog"),
            "{helper} has a fixed safe search path"
        );
        assert!(
            !declaration.contains("SECURITY DEFINER"),
            "{helper} remains invoker-security"
        );
        assert!(UP.contains(&format!("ALTER FUNCTION public.{helper}")));
        assert!(UP.contains("OWNER TO migration_owner"));
        assert!(UP.contains(&format!("REVOKE ALL ON FUNCTION public.{helper}")));
    }
    let binding_acl = UP
        .split("REVOKE ALL ON FUNCTION public.owner_beta_simulation_version_binding_matches")
        .nth(1)
        .expect("binding helper ACL exists")
        .split("-- The key namespace")
        .next()
        .expect("binding helper ACL section ends before namespace helper");
    assert!(binding_acl.contains("FROM PUBLIC, app, worker, admin, audit_writer, research_writer"));
    assert!(binding_acl.contains("TO app, worker"));
    let namespace_acl = UP
        .split("REVOKE ALL ON FUNCTION public.owner_beta_simulation_version_namespace_matches")
        .nth(1)
        .expect("namespace helper ACL exists")
        .split("ALTER TABLE public.owner_beta_simulation_runs")
        .next()
        .expect("namespace helper ACL section ends before run constraints");
    assert!(
        namespace_acl.contains("FROM PUBLIC, app, worker, admin, audit_writer, research_writer")
    );
    assert!(!namespace_acl.contains("GRANT EXECUTE"));

    let binding = function_body(UP, "owner_beta_simulation_version_binding_matches");
    assert!(binding.contains("p_schema = 'owner-beta-price-only-simulation-job-v1'"));
    assert!(binding.contains("p_schema = 'owner-beta-price-only-simulation-job-v2'"));
    for strategy in [
        "buy_and_hold",
        "trend_following",
        "relative_momentum",
        "dual_momentum",
        "inverse_volatility",
    ] {
        assert!(
            binding.contains(&format!("WHEN '{strategy}'")),
            "{strategy} branch"
        );
    }
    assert!(binding.contains("AND p_config_json ="));
    assert!(binding.contains("AND p_config_sha256 ="));

    for trigger in [
        "owner_beta_simulation_jobs_insert_guard",
        "owner_beta_simulation_runs_validate_binding",
    ] {
        let declaration = function_declaration(UP, trigger);
        assert!(
            declaration.contains("SECURITY DEFINER"),
            "{trigger} remains privileged"
        );
        assert!(
            declaration.contains("SET search_path = pg_catalog, pg_temp"),
            "{trigger} has its original safe search path"
        );
        assert!(UP.contains(&format!(
            "ALTER FUNCTION public.{trigger}() OWNER TO migration_owner"
        )));
        assert!(UP.contains(&format!("REVOKE ALL ON FUNCTION public.{trigger}()")));
        assert!(
            !UP.contains(&format!("GRANT EXECUTE ON FUNCTION public.{trigger}()")),
            "{trigger} gains no execution grant"
        );
    }
}

#[test]
fn down_refuses_v2_history_before_restoring_functions_or_constraints() {
    let locks = DOWN.find("LOCK TABLE public.jobs IN SHARE MODE").unwrap();
    let no_force = DOWN
        .find("ALTER TABLE public.jobs NO FORCE ROW LEVEL SECURITY")
        .unwrap();
    let guard = DOWN
        .find("DO $owner_beta_simulation_v2_history_guard$")
        .unwrap();
    let first_function = DOWN.find("CREATE OR REPLACE FUNCTION").unwrap();
    let restore_constraint = DOWN
        .find("DROP CONSTRAINT owner_beta_simulation_runs_version_binding_check")
        .unwrap();
    assert!(locks < no_force && no_force < guard);
    assert!(guard < first_function && guard < restore_constraint);

    let guard_source = &DOWN[guard..first_function];
    for required in [
        "owner_beta_simulation_runs",
        "public.jobs",
        "public.job_attempts",
        "owner_beta_simulation_result_pairs",
        "owner-beta-price-only-simulation-job-v2",
        "owner-beta:price-only-simulation:v2:%",
        "ERRCODE = '55000'",
    ] {
        assert!(guard_source.contains(required), "{required}");
    }
    assert!(!guard_source.contains("job.status"));
    assert!(DOWN.contains("ALTER TABLE public.jobs FORCE ROW LEVEL SECURITY"));
    assert!(
        DOWN.contains("ALTER TABLE public.owner_beta_simulation_runs FORCE ROW LEVEL SECURITY")
    );
    assert!(DOWN.contains("DROP FUNCTION public.owner_beta_simulation_version_binding_matches("));
    assert!(DOWN.contains(
        "DROP FUNCTION public.owner_beta_simulation_version_namespace_matches(text, text, text)"
    ));
    let restored_constraints = DOWN
        .find("ADD CONSTRAINT owner_beta_simulation_runs_strategy_id_check")
        .unwrap();
    let drop_namespace = DOWN
        .find("DROP FUNCTION public.owner_beta_simulation_version_namespace_matches")
        .unwrap();
    let drop_binding = DOWN
        .find("DROP FUNCTION public.owner_beta_simulation_version_binding_matches")
        .unwrap();
    assert!(
        restored_constraints < drop_namespace && drop_namespace < drop_binding,
        "new helper functions are dropped after the original constraints are restored"
    );

    // The old v1 down guard remains byte-for-byte outside the new migration.
    assert!(ORIGINAL_DOWN.contains("empty run and pair history"));
}

#[test]
fn only_the_version_bound_functions_and_run_checks_change_in_up() {
    assert!(UP.contains("owner_beta_simulation_runs_version_binding_check"));
    assert!(!UP.contains(
        "CREATE OR REPLACE FUNCTION public.owner_beta_simulation_jobs_validate_insert_payload"
    ));
    assert!(
        !UP.contains(
            "CREATE OR REPLACE FUNCTION public.owner_beta_simulation_pairs_validate_insert"
        )
    );
    assert!(!UP.contains("CREATE OR REPLACE FUNCTION public.owner_beta_simulation_deferred_guard"));
    assert!(!UP.contains("CREATE OR REPLACE FUNCTION public.owner_beta_simulation_attempt_guard"));
    assert!(UP.contains("OWNER TO migration_owner"));
    assert!(
        UP.contains("REVOKE ALL ON FUNCTION public.owner_beta_simulation_version_binding_matches")
    );
    assert!(UP.contains("TO app, worker"));
    assert!(!UP.contains("SECURITY DEFINER\nSET search_path = pg_catalog\nAS $version_binding$"));

    for field in [
        "owner_beta_simulation_runs_input_kind_check",
        "owner_beta_simulation_runs_capability_check",
        "owner_beta_simulation_runs_audience_check",
        "owner_beta_simulation_runs_vendor_snapshot_check",
        "owner_beta_simulation_runs_strict_pit_check",
        "owner_beta_simulation_runs_action_count_check",
        "owner_beta_simulation_runs_candidate_hash_check",
        "owner_beta_simulation_runs_artifact_hash_check",
        "owner_beta_simulation_runs_stage5_hash_check",
        "owner_beta_simulation_runs_action_hash_check",
        "owner_beta_simulation_runs_approval_hash_check",
        "owner_beta_simulation_runs_bars_hash_check",
    ] {
        assert!(ORIGINAL_UP.contains(field), "original field guard {field}");
        assert!(
            !UP.contains(&format!("DROP CONSTRAINT {field}")),
            "preserve {field}"
        );
    }
}
