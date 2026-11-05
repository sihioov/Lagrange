//! Root-protected source observations, shared Python/Rust wire and DB links.
//! Root/kernel/Docker observations are trusted, not independently re-observed.
//! Parsing bytes alone never constructs a filesystem capability or eligibility.
use chrono::{DateTime, Datelike, NaiveDate};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};
use uuid::Uuid;

pub const SOURCE_LIMIT: usize = 65_536;
pub const SESSION_LIMIT: usize = 262_144;
pub const ELIGIBILITY_LIMIT: usize = 1_048_576;
pub const MAX_DEPTH: usize = 16;
// Generated embedding of the one schema in docs/contracts. The cross-language
// test checks exact JSON equality; no Docker build-context expansion is needed.
pub const SCHEMA_JSON: &str = r##"{"$defs":{"action":{"additionalProperties":false,"properties":{"admitted":{"const":false},"batch_id":{"anyOf":[{"type":"null"},{"format":"uuid","maxLength":512,"minLength":1,"type":"string"}]},"claim_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"end":{"anyOf":[{"type":"null"},{"format":"date","maxLength":512,"minLength":1,"type":"string"}]},"outcome":{"enum":["RAW_CAPTURE_INDEPENDENTLY_VERIFIED","VERIFIED_PRIOR_CAPTURE","ALREADY_COVERED"]},"raw_only":{"const":true},"reserved_wire_gets":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"start":{"anyOf":[{"type":"null"},{"format":"date","maxLength":512,"minLength":1,"type":"string"}]}},"required":["outcome","start","end","batch_id","reserved_wire_gets","raw_only","admitted","claim_sha256"],"type":"object"},"admission":{"additionalProperties":false,"properties":{"admission_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"original_set_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"readback_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"source_event_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["admission_sha256","source_event_sha256","original_set_sha256","readback_sha256"],"type":"object"},"arm":{"additionalProperties":false,"properties":{"boot_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"generation":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"nonce":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"parent":{"$ref":"#/$defs/process"}},"required":["boot_id","generation","nonce","parent"],"type":"object"},"binding":{"additionalProperties":false,"properties":{"calendar_dir":{"maxLength":512,"minLength":1,"type":"string"},"calendar_file":{"maxLength":512,"minLength":1,"type":"string"},"cohort":{"$ref":"#/$defs/cohort"},"daily_config_file":{"maxLength":512,"minLength":1,"type":"string"},"daily_config_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"data_root":{"maxLength":512,"minLength":1,"type":"string"},"database":{"maxLength":512,"minLength":1,"type":"string"},"evidence_root":{"maxLength":512,"minLength":1,"type":"string"},"lock_file":{"maxLength":512,"minLength":1,"type":"string"},"max_source_seconds":{"maximum":43200,"minimum":1,"type":"integer"},"network":{"maxLength":512,"minLength":1,"type":"string"},"production_env_file":{"maxLength":512,"minLength":1,"type":"string"},"production_env_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"project":{"maxLength":512,"minLength":1,"type":"string"},"raw_owner_uid":{"enum":[10001,10002]},"release_manifest_file":{"maxLength":512,"minLength":1,"type":"string"},"release_root":{"maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-witness-binding-v1"},"secret_root":{"maxLength":512,"minLength":1,"type":"string"},"state_dir":{"maxLength":512,"minLength":1,"type":"string"}},"required":["evidence_root","release_root","production_env_file","calendar_dir","calendar_file","release_manifest_file","data_root","daily_config_file","secret_root","database","project","network","schema","cohort","raw_owner_uid","max_source_seconds","daily_config_sha256","production_env_sha256","state_dir","lock_file"],"type":"object"},"chain":{"additionalProperties":false,"properties":{"events":{"items":{"$ref":"#/$defs/event"},"maxItems":64,"type":"array"},"schema":{"const":"owner-beta-paper-source-chain-v1"}},"required":["schema","events"],"type":"object"},"clock":{"additionalProperties":false,"properties":{"boottime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"monotonic_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"}},"required":["realtime_ns","boottime_ns","monotonic_ns"],"type":"object"},"cohort":{"additionalProperties":false,"properties":{"activation_epoch":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"calendar_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"cohort_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"config_epoch":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"entitlement_reference":{"maxLength":512,"minLength":1,"type":"string"},"entitlement_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"helper_image_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"owner_user_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"policy_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"release_manifest_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"research_image_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"semantic_config_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"strategy_config_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"strategy_config_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"verifier_revision":{"format":"commit","maxLength":512,"minLength":1,"type":"string"}},"required":["owner_user_id","policy_sha256","activation_epoch","strategy_config_id","config_epoch","strategy_config_sha256","entitlement_reference","entitlement_sha256","cohort_sha256","release_manifest_sha256","research_image_id","helper_image_id","semantic_config_sha256","calendar_sha256","verifier_revision"],"type":"object"},"container":{"additionalProperties":false,"properties":{"argv_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"client":{"$ref":"#/$defs/process"},"config_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"created_at":{"format":"date-time","maxLength":512,"minLength":1,"type":"string"},"exit_code":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"finished_at":{"format":"date-time","maxLength":512,"minLength":1,"type":"string"},"id":{"format":"hex64","maxLength":512,"minLength":1,"type":"string"},"image_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"oom_killed":{"type":"boolean"},"phase":{"enum":["precheck","capture","action","reserve","link-admission","link-run","read"]},"started_at":{"format":"date-time","maxLength":512,"minLength":1,"type":"string"}},"required":["phase","id","image_id","config_sha256","created_at","started_at","finished_at","exit_code","oom_killed","argv_sha256","client"],"type":"object"},"database":{"additionalProperties":false,"properties":{"admission":{"anyOf":[{"type":"null"},{"additionalProperties":false,"properties":{"admission_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"linked_at":{"format":"db-timestamp","maxLength":512,"minLength":1,"type":"string"},"original_set_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"source_event_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["slot_id","source_event_sha256","original_set_sha256","admission_sha256","linked_at"],"type":"object"}]},"admitted_at":{"anyOf":[{"type":"null"},{"format":"db-timestamp","maxLength":512,"minLength":1,"type":"string"}]},"attempt_id":{"anyOf":[{"type":"null"},{"format":"uuid","maxLength":512,"minLength":1,"type":"string"}]},"attempts":{"anyOf":[{"type":"null"},{"items":{"additionalProperties":false,"properties":{"attempt_no":{"maximum":3,"minimum":1,"type":"integer"},"finished_at":{"anyOf":[{"type":"null"},{"format":"db-timestamp","maxLength":512,"minLength":1,"type":"string"}]},"id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"outcome":{"enum":["RUNNING","SUCCEEDED","FAILED","ORPHANED"]},"started_at":{"format":"db-timestamp","maxLength":512,"minLength":1,"type":"string"}},"required":["id","attempt_no","outcome","started_at","finished_at"],"type":"object"},"maxItems":3,"type":"array"}]},"candidate_admission_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"candidate_pins":{"anyOf":[{"type":"null"},{"additionalProperties":false,"properties":{"admission_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"content_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"manifest_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"policy_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["content_sha256","manifest_sha256","admission_sha256","policy_sha256"],"type":"object"}]},"candidate_run_id":{"anyOf":[{"type":"null"},{"format":"uuid","maxLength":512,"minLength":1,"type":"string"}]},"factor_snapshot_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"finished_at":{"anyOf":[{"type":"null"},{"format":"db-timestamp","maxLength":512,"minLength":1,"type":"string"}]},"job_id":{"anyOf":[{"type":"null"},{"format":"uuid","maxLength":512,"minLength":1,"type":"string"}]},"pins":{"anyOf":[{"type":"null"},{"additionalProperties":false,"properties":{"admission_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"content_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"manifest_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"policy_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["content_sha256","manifest_sha256","admission_sha256","policy_sha256"],"type":"object"}]},"run":{"anyOf":[{"type":"null"},{"additionalProperties":false,"properties":{"attempt_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"job_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"linked_at":{"format":"db-timestamp","maxLength":512,"minLength":1,"type":"string"},"run_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["slot_id","run_id","job_id","attempt_id","linked_at"],"type":"object"}]},"schema":{"const":"owner-beta-paper-db-v1"},"slot":{"additionalProperties":false,"properties":{"activation_epoch":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"cohort":{"$ref":"#/$defs/cohort"},"cohort_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"owner_user_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"policy_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"reserved_at":{"format":"db-timestamp","maxLength":512,"minLength":1,"type":"string"},"root_event_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"session_date":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["slot_id","reservation_id","root_event_sha256","owner_user_id","policy_sha256","activation_epoch","session_date","cohort_sha256","cohort","reserved_at"],"type":"object"},"target_snapshot_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"terminal":{"enum":[null,"QUEUED","RUNNING","SUCCEEDED","FAILED","CANCELED"]}},"required":["schema","slot","admission","run","candidate_admission_sha256","candidate_run_id","terminal","job_id","attempt_id","attempts","factor_snapshot_sha256","target_snapshot_sha256","candidate_pins","pins","admitted_at","finished_at"],"type":"object"},"db":{"additionalProperties":false,"properties":{"readback_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["readback_sha256"],"type":"object"},"empty":{"additionalProperties":false,"properties":{},"required":[],"type":"object"},"event":{"oneOf":[{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/empty"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"STARTED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/empty"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"ARM_INTENT"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/empty"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"ARMED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/empty"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"RESERVED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/db"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"DB_RESERVED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/launch"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"LAUNCH_INTENT"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/running"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"RUNNING"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/container"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"CONTAINER"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/raw"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"RAW_SOURCE"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/source"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"SOURCE_FINISHED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/admission"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"ADMISSION_LINKED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/run"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"RUN_LINKED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/empty"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"LINKED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"},{"additionalProperties":false,"properties":{"arm":{"$ref":"#/$defs/arm"},"cohort":{"$ref":"#/$defs/cohort"},"data":{"$ref":"#/$defs/failure"},"due_realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"invocation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"kind":{"const":"FAILED"},"observed":{"$ref":"#/$defs/clock"},"previous_sha256":{"anyOf":[{"type":"null"},{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}]},"reservation_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-source-event-v1"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"session":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"slot_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["schema","slot_id","session","due_realtime_ns","cohort","arm","reservation_id","invocation_id","sequence","previous_sha256","observed","kind","data"],"type":"object"}]},"failure":{"additionalProperties":false,"properties":{"code":{"enum":["UNKNOWN","CLOCK_INVALID","MISSED_LATE","MISSED_UNARMED","ALREADY_CLAIMED","ORIGIN_UNPROVEN","SOURCE_INVALID","DB_UNAVAILABLE","LINK_PENDING","CONFIG_CHANGED","INTERRUPTED"]}},"required":["code"],"type":"object"},"launch":{"additionalProperties":false,"properties":{"argv_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["argv_sha256"],"type":"object"},"original":{"additionalProperties":false,"properties":{"batch_date":{"format":"date","maxLength":512,"minLength":1,"type":"string"},"batch_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"batch_json_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"file_name":{"format":"leaf","maxLength":512,"minLength":1,"type":"string"},"manifest_line_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"market":{"const":"kr"},"provider":{"enum":["kis","kis-calendar"]},"retrieved_at":{"format":"date-time","maxLength":512,"minLength":1,"type":"string"},"size_bytes":{"maximum":9223372036854775807,"minimum":1,"type":"integer"},"wire_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}},"required":["provider","market","batch_id","batch_date","file_name","wire_sha256","size_bytes","batch_json_sha256","manifest_line_sha256","retrieved_at"],"type":"object"},"process":{"additionalProperties":false,"properties":{"pid":{"maximum":9223372036854775807,"minimum":1,"type":"integer"},"start_ticks":{"maximum":9223372036854775807,"minimum":1,"type":"integer"}},"required":["pid","start_ticks"],"type":"object"},"raw":{"additionalProperties":false,"properties":{"batch_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"calendar_batch_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"calendar_claim_preexisting":{"type":"boolean"},"reused_existing_source":{"type":"boolean"},"target_date":{"format":"date","maxLength":512,"minLength":1,"type":"string"}},"required":["batch_id","target_date","reused_existing_source","calendar_claim_preexisting","calendar_batch_id"],"type":"object"},"run":{"additionalProperties":false,"properties":{"attempt_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"job_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"readback_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"run_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"}},"required":["run_id","job_id","attempt_id","readback_sha256"],"type":"object"},"running":{"additionalProperties":false,"properties":{"child":{"$ref":"#/$defs/process"}},"required":["child"],"type":"object"},"source":{"additionalProperties":false,"properties":{"action":{"$ref":"#/$defs/action"},"original_set_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"originals":{"items":{"$ref":"#/$defs/original"},"maxItems":23,"type":"array"},"wrapper_exit":{"const":0}},"required":["wrapper_exit","original_set_sha256","originals","action"],"type":"object"}},"$id":"urn:lagrange:owner-beta-paper-source-evidence-v1","$schema":"https://json-schema.org/draft/2020-12/schema","anyOf":[{"$ref":"#/$defs/event"},{"$ref":"#/$defs/chain"},{"$ref":"#/$defs/binding"},{"$ref":"#/$defs/database"}],"description":"Canonical UTF-8 JSON, sorted keys, no whitespace or floats. Source max 65536 bytes, chain max 262144 bytes, JSON depth 16. Custom formats sha256, hex64, commit and leaf are enforced by both readers. Clock integers are nanoseconds, due is 16:30 Asia/Seoul. Root/kernel/Docker trusted; SQL never attests scheduler origin.","title":"Root source observations; decoding never grants authority"}"##;
pub const CURRENT_SCHEMA_JSON: &str = r######"{"$defs":{"binding":{"additionalProperties":false,"properties":{"calendar_dir":{"maxLength":512,"minLength":1,"type":"string"},"calendar_file":{"maxLength":512,"minLength":1,"type":"string"},"cohort":{"$ref":"#/$defs/cohort"},"daily_config_file":{"maxLength":512,"minLength":1,"type":"string"},"daily_config_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"data_root":{"maxLength":512,"minLength":1,"type":"string"},"database":{"maxLength":512,"minLength":1,"type":"string"},"evidence_root":{"maxLength":512,"minLength":1,"type":"string"},"lock_file":{"maxLength":512,"minLength":1,"type":"string"},"max_source_seconds":{"maximum":43200,"minimum":1,"type":"integer"},"network":{"maxLength":512,"minLength":1,"type":"string"},"production_env_file":{"maxLength":512,"minLength":1,"type":"string"},"production_env_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"project":{"maxLength":512,"minLength":1,"type":"string"},"raw_owner_uid":{"enum":[10001,10002]},"release_manifest_file":{"maxLength":512,"minLength":1,"type":"string"},"release_root":{"maxLength":512,"minLength":1,"type":"string"},"schema":{"const":"owner-beta-paper-witness-binding-v1"},"secret_root":{"maxLength":512,"minLength":1,"type":"string"},"state_dir":{"maxLength":512,"minLength":1,"type":"string"}},"required":["evidence_root","release_root","production_env_file","calendar_dir","calendar_file","release_manifest_file","data_root","daily_config_file","secret_root","database","project","network","schema","cohort","raw_owner_uid","max_source_seconds","daily_config_sha256","production_env_sha256","state_dir","lock_file"],"type":"object"},"clock":{"additionalProperties":false,"properties":{"boottime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"monotonic_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"realtime_ns":{"maximum":9223372036854775807,"minimum":0,"type":"integer"}},"required":["realtime_ns","boottime_ns","monotonic_ns"],"type":"object"},"cohort":{"additionalProperties":false,"properties":{"activation_epoch":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"calendar_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"cohort_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"config_epoch":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"entitlement_reference":{"maxLength":512,"minLength":1,"type":"string"},"entitlement_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"helper_image_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"owner_user_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"policy_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"release_manifest_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"research_image_id":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"semantic_config_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"strategy_config_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"strategy_config_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"verifier_revision":{"format":"commit","maxLength":512,"minLength":1,"type":"string"}},"required":["owner_user_id","policy_sha256","activation_epoch","strategy_config_id","config_epoch","strategy_config_sha256","entitlement_reference","entitlement_sha256","cohort_sha256","release_manifest_sha256","research_image_id","helper_image_id","semantic_config_sha256","calendar_sha256","verifier_revision"],"type":"object"},"process":{"additionalProperties":false,"properties":{"pid":{"maximum":9223372036854775807,"minimum":1,"type":"integer"},"start_ticks":{"maximum":9223372036854775807,"minimum":1,"type":"integer"}},"required":["pid","start_ticks"],"type":"object"}},"$id":"urn:lagrange:owner-beta-paper-current-binding-v1","$schema":"https://json-schema.org/draft/2020-12/schema","additionalProperties":false,"allOf":[{"else":{"anyOf":[{"properties":{"binding":{"type":"null"},"binding_sha256":{"type":"null"}}},{"properties":{"binding":{"$ref":"#/$defs/binding"},"binding_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"}}}],"properties":{"reason":{"enum":["STARTING","STOPPED","CONFIG_CHANGED","CLOCK_INVALID","REPLACED","UNKNOWN"]}}},"if":{"properties":{"status":{"const":"CURRENT"}}},"then":{"properties":{"binding":{"$ref":"#/$defs/binding"},"binding_sha256":{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},"reason":{"const":"NONE"}}}}],"description":"Canonical UTF-8 <=65536 bytes, depth<=16. CURRENT requires NONE and non-null binding/digest; INVALID requires non-NONE and matched binding/digest nullability. Semantic hash, boot/clocks, protected metadata and live lock are runtime requirements, not JSON authority.","properties":{"binding":{"anyOf":[{"$ref":"#/$defs/binding"},{"type":"null"}]},"binding_sha256":{"anyOf":[{"format":"sha256","maxLength":512,"minLength":1,"type":"string"},{"type":"null"}]},"boot_id":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"generation":{"format":"uuid","maxLength":512,"minLength":1,"type":"string"},"observed":{"$ref":"#/$defs/clock"},"publisher":{"$ref":"#/$defs/process"},"reason":{"enum":["NONE","STARTING","STOPPED","CONFIG_CHANGED","CLOCK_INVALID","REPLACED","UNKNOWN"]},"schema":{"const":"owner-beta-paper-current-binding-v1","maxLength":512,"minLength":1,"type":"string"},"sequence":{"maximum":9223372036854775807,"minimum":0,"type":"integer"},"status":{"enum":["CURRENT","INVALID"]}},"required":["schema","status","reason","generation","boot_id","publisher","sequence","observed","binding_sha256","binding"],"title":"Untrusted current selector envelope; protected live OFD fence required separately","type":"object"}"######;
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("PAPER_SOURCE_EVIDENCE_REJECTED")]
pub struct EvidenceError;
type Result<T> = std::result::Result<T, EvidenceError>;
fn require(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(EvidenceError) }
}
pub fn canonical(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| EvidenceError)
}
pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn bounded_depth(value: &Value, depth: usize) -> Result<()> {
    require(depth <= MAX_DEPTH)?;
    match value {
        Value::Object(v) => {
            for child in v.values() {
                bounded_depth(child, depth + 1)?;
            }
        }
        Value::Array(v) => {
            for child in v {
                bounded_depth(child, depth + 1)?;
            }
        }
        Value::Number(v) => require(v.is_i64())?,
        _ => {}
    }
    Ok(())
}
pub fn decode_canonical(bytes: &[u8], limit: usize) -> Result<Value> {
    require(!bytes.is_empty() && bytes.len() <= limit)?;
    let value: Value = serde_json::from_slice(bytes).map_err(|_| EvidenceError)?;
    bounded_depth(&value, 1)?;
    // Also rejects duplicate keys, lossy integer parsing and trailing whitespace.
    require(canonical(&value)? == bytes)?;
    Ok(value)
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn format_ok(value: &str, kind: &str) -> bool {
    match kind {
        "sha256" => value.strip_prefix("sha256:").is_some_and(|v| hex(v, 64)),
        "hex64" => hex(value, 64),
        "commit" => hex(value, 40),
        "uuid" => Uuid::parse_str(value).is_ok_and(|v| !v.is_nil() && v.to_string() == value),
        "date" => {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok_and(|v| v.to_string() == value)
        }
        "db-timestamp" => {
            value.len() <= 35 && DateTime::parse_from_rfc3339(value).is_ok_and(|v| v.year() > 1970)
        }
        "date-time" => {
            value.ends_with('Z')
                && value.as_bytes().get(10) == Some(&b'T')
                && value.len() <= 30
                && DateTime::parse_from_rfc3339(value)
                    .is_ok_and(|v| v.year() > 1970 && v.offset().local_minus_utc() == 0)
        }
        "leaf" => {
            !value.is_empty()
                && value.len() <= 192
                && value != "batch.json"
                && value.as_bytes()[0].is_ascii_alphanumeric()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        }
        _ => false,
    }
}
fn validate(value: &Value, rule: &Value, schema: &Value) -> Result<()> {
    if let Some(reference) = rule["$ref"].as_str() {
        return validate(
            value,
            &schema["$defs"][reference.rsplit('/').next().ok_or(EvidenceError)?],
            schema,
        );
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = rule[key].as_array() {
            let count = branches
                .iter()
                .filter(|r| validate(value, r, schema).is_ok())
                .count();
            return require(if key == "oneOf" {
                count == 1
            } else {
                count > 0
            });
        }
    }
    if let Some(constant) = rule.get("const") {
        require(value == constant)?;
    }
    if let Some(values) = rule["enum"].as_array() {
        require(values.contains(value))?;
    }
    match rule["type"].as_str() {
        Some("object") => {
            let obj = value.as_object().ok_or(EvidenceError)?;
            let keys = rule["required"].as_array().ok_or(EvidenceError)?;
            require(obj.len() == keys.len())?;
            for key in keys {
                let k = key.as_str().ok_or(EvidenceError)?;
                validate(
                    obj.get(k).ok_or(EvidenceError)?,
                    &rule["properties"][k],
                    schema,
                )?;
            }
        }
        Some("array") => {
            let arr = value.as_array().ok_or(EvidenceError)?;
            require(arr.len() as u64 <= rule["maxItems"].as_u64().ok_or(EvidenceError)?)?;
            for child in arr {
                validate(child, &rule["items"], schema)?;
            }
        }
        Some("integer") => {
            let number = value.as_i64().ok_or(EvidenceError)?;
            require(
                number >= rule["minimum"].as_i64().ok_or(EvidenceError)?
                    && number <= rule["maximum"].as_i64().ok_or(EvidenceError)?,
            )?;
        }
        Some("boolean") => require(value.is_boolean())?,
        Some("null") => require(value.is_null())?,
        Some("string") => {
            let text = value.as_str().ok_or(EvidenceError)?;
            require(
                text.is_ascii()
                    && !text.bytes().any(|b| b < 32)
                    && text.len() as u64 >= rule["minLength"].as_u64().ok_or(EvidenceError)?
                    && text.len() as u64 <= rule["maxLength"].as_u64().ok_or(EvidenceError)?,
            )?;
            if let Some(kind) = rule["format"].as_str() {
                require(format_ok(text, kind))?;
            }
        }
        None => (),
        _ => return Err(EvidenceError),
    }
    Ok(())
}
pub fn validate_wire(value: &Value) -> Result<()> {
    let schema: Value = serde_json::from_str(SCHEMA_JSON).map_err(|_| EvidenceError)?;
    validate(value, &schema, &schema)
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().ok_or(EvidenceError)
}
fn integer(v: &Value, key: &str) -> Result<i64> {
    v[key].as_i64().ok_or(EvidenceError)
}
pub fn due_ns(session: &str) -> Result<i64> {
    NaiveDate::parse_from_str(session, "%Y-%m-%d")
        .ok()
        .and_then(|v| v.and_hms_opt(7, 30, 0))
        .and_then(|v| v.and_utc().timestamp_nanos_opt())
        .ok_or(EvidenceError)
}
pub fn slot_id(cohort: &Value, session: &str) -> Result<String> {
    Ok(digest(&canonical(
        &json!({"schema":"owner-beta-paper-slot-v1","owner":cohort["owner_user_id"],"policy":cohort["policy_sha256"],"cohort":cohort["cohort_sha256"],"session":session,"due_realtime_ns":due_ns(session)?}),
    )?))
}
fn clock_step(before: &Value, after: &Value) -> Result<()> {
    let mut delta = [0_i64; 3];
    for (i, k) in ["realtime_ns", "boottime_ns", "monotonic_ns"]
        .iter()
        .enumerate()
    {
        delta[i] = integer(after, k)?
            .checked_sub(integer(before, k)?)
            .ok_or(EvidenceError)?;
        require(delta[i] >= 0)?;
    }
    require(
        delta[0].abs_diff(delta[1]) <= 1_000_000_000
            && delta[1].abs_diff(delta[2]) <= 1_000_000_000,
    )
}
pub fn original_set_hash(originals: &[Value]) -> Result<String> {
    require(originals.len() == 23)?;
    let mut keys = Vec::new();
    for value in originals {
        keys.push(
            ["provider", "market", "batch_id", "file_name", "wire_sha256"]
                .into_iter()
                .map(|k| text(value, k))
                .collect::<Result<Vec<_>>>()?
                .join("|"),
        );
    }
    let identities = originals
        .iter()
        .map(|o| {
            ["provider", "market", "batch_id", "file_name"].map(|k| o[k].as_str().unwrap_or(""))
        })
        .collect::<Vec<_>>();
    require(identities.windows(2).all(|pair| pair[0] < pair[1]))?;
    Ok(digest((keys.join("\n") + "\n").as_bytes()))
}
/// Validates observations only. This public pure function is useful for shared
/// fixtures; it cannot attest root ownership, current rights or actual DB state.
pub fn verify_chain(events: &[Value], terminal: Option<&str>) -> Result<()> {
    require(!events.is_empty() && events.len() <= 64)?;
    let wrapper = json!({"schema":"owner-beta-paper-source-chain-v1","events":events});
    require(canonical(&wrapper)?.len() <= SESSION_LIMIT)?;
    validate_wire(&wrapper)?;
    let stages = [
        "STARTED",
        "ARM_INTENT",
        "ARMED",
        "RESERVED",
        "DB_RESERVED",
        "LAUNCH_INTENT",
        "RUNNING",
        "SOURCE_FINISHED",
        "ADMISSION_LINKED",
        "RUN_LINKED",
        "LINKED",
    ];
    let mut stage = 0_usize;
    let mut phases = Vec::new();
    let mut containers = BTreeSet::new();
    let mut raw: Option<&Value> = None;
    let mut source: Option<(&Value, String)> = None;
    for (index, event) in events.iter().enumerate() {
        require(
            canonical(event)?.len() <= SOURCE_LIMIT && integer(event, "sequence")? == index as i64,
        )?;
        require(
            event["previous_sha256"]
                == if index == 0 {
                    Value::Null
                } else {
                    json!(digest(&canonical(&events[index - 1])?))
                },
        )?;
        for k in [
            "slot_id",
            "session",
            "due_realtime_ns",
            "cohort",
            "arm",
            "reservation_id",
            "invocation_id",
        ] {
            require(event[k] == events[0][k])?;
        }
        let session = text(event, "session")?;
        let due = due_ns(session)?;
        let now = integer(&event["observed"], "realtime_ns")?;
        require(
            integer(event, "due_realtime_ns")? == due
                && text(event, "slot_id")? == slot_id(&event["cohort"], session)?,
        )?;
        if index > 0 {
            clock_step(&events[index - 1]["observed"], &event["observed"])?;
            clock_step(&events[0]["observed"], &event["observed"])?;
        }
        let kind = text(event, "kind")?;
        let data = &event["data"];
        match kind {
            "FAILED" => require(index == events.len() - 1 && terminal.is_none())?,
            "CONTAINER" => {
                require(
                    (4..=10).contains(&stage)
                        && data["exit_code"] == 0
                        && data["oom_killed"] == false,
                )?;
                require(containers.insert(text(data, "id")?))?;
                let phase = text(data, "phase")?;
                let research = ["precheck", "capture", "action"].contains(&phase);
                require(
                    data["image_id"]
                        == event["cohort"][if research {
                            "research_image_id"
                        } else {
                            "helper_image_id"
                        }],
                )?;
                if research {
                    require(
                        stage == 7
                            && ["precheck", "capture", "action"].get(phases.len()) == Some(&phase),
                    )?;
                    phases.push(phase);
                }
            }
            "RAW_SOURCE" => {
                require(
                    stage == 7
                        && raw.is_none()
                        && phases == ["precheck", "capture"]
                        && data["target_date"] == event["session"]
                        && data["reused_existing_source"] == false
                        && data["calendar_claim_preexisting"] == false
                        && data["batch_id"] != data["calendar_batch_id"],
                )?;
                raw = Some(data);
            }
            _ => {
                require(stages.get(stage) == Some(&kind))?;
                stage += 1;
                if ["ARM_INTENT", "ARMED"].contains(&kind) {
                    require(due.checked_sub(now).ok_or(EvidenceError)? >= 60_000_000_000)?;
                }
                if ["RESERVED", "DB_RESERVED", "LAUNCH_INTENT", "RUNNING"].contains(&kind) {
                    require(now >= due && now <= due + 60_000_000_000)?;
                }
                if kind == "RUNNING" {
                    require(
                        data["child"]["pid"] != event["arm"]["parent"]["pid"]
                            && integer(&data["child"], "start_ticks")?
                                >= integer(&event["arm"]["parent"], "start_ticks")?,
                    )?;
                }
                if kind == "SOURCE_FINISHED" {
                    let raw = raw.ok_or(EvidenceError)?;
                    require(phases == ["precheck", "capture", "action"])?;
                    let originals = data["originals"].as_array().ok_or(EvidenceError)?;
                    require(
                        data["original_set_sha256"] == original_set_hash(originals)?
                            && originals
                                .iter()
                                .all(|o| o["batch_date"] == event["session"]),
                    )?;
                    require(
                        originals
                            .iter()
                            .filter(|o| o["batch_id"] == raw["batch_id"])
                            .count()
                            == 22
                            && originals
                                .iter()
                                .filter(|o| o["batch_id"] == raw["calendar_batch_id"])
                                .count()
                                == 1,
                    )?;
                    let launched = events
                        .iter()
                        .find(|e| e["kind"] == "RUNNING")
                        .ok_or(EvidenceError)?;
                    let launch_ns = integer(&launched["observed"], "realtime_ns")?;
                    for original in originals {
                        let retrieved =
                            DateTime::parse_from_rfc3339(text(original, "retrieved_at")?)
                                .map_err(|_| EvidenceError)?
                                .timestamp_nanos_opt()
                                .ok_or(EvidenceError)?;
                        require(
                            retrieved >= launch_ns - 1_000_000_000
                                && retrieved <= now + 1_000_000_000,
                        )?;
                    }
                    source = Some((data, digest(&canonical(event)?)));
                }
                if kind == "ADMISSION_LINKED" {
                    let (s, h) = source.as_ref().ok_or(EvidenceError)?;
                    require(
                        data["source_event_sha256"] == *h
                            && data["original_set_sha256"] == s["original_set_sha256"],
                    )?;
                }
                if kind == "LINKED" {
                    require(now < due + 27_000_000_000_000)?;
                }
            }
        }
    }
    if let Some(last) = terminal {
        require(events.last().is_some_and(|e| e["kind"] == last))?;
    }
    Ok(())
}

/// Holds root-owned originals open. Consumers must also check current DB state
/// and sealed Daily/Raw evidence. No Serialize/Deserialize capability bypass.
pub struct ProtectedSourceChain {
    events: Vec<Value>,
    #[cfg(target_os = "linux")]
    files: Vec<(std::fs::File, Vec<u8>)>,
    directory: std::path::PathBuf,
}
impl ProtectedSourceChain {
    pub fn events(&self) -> &[Value] {
        &self.events
    }
    pub fn first(&self) -> &Value {
        &self.events[0]
    }
    pub fn event(&self, kind: &str) -> Result<&Value> {
        self.events
            .iter()
            .find(|e| e["kind"] == kind)
            .ok_or(EvidenceError)
    }
    pub fn open(root: &Path, slot: &str) -> Result<Self> {
        require(format_ok(slot, "sha256"))?;
        #[cfg(target_os = "linux")]
        {
            let io = CurrentIo::new(CurrentMode::Legacy);
            let (result, data) = open_source_with(&io, root, slot);
            let _ = io.finish_trace();
            legacy_current(result)?;
            Ok(Self {
                events: data.events,
                files: data
                    .files
                    .into_iter()
                    .map(|(file, bytes)| (file.legacy(), bytes))
                    .collect(),
                directory: data.directory,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = root;
            Err(EvidenceError)
        }
    }
    pub fn revalidate(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let io = CurrentIo::new(CurrentMode::Legacy);
            let result = revalidate_source_with(&io, &self.directory, self.files.len(), |index| {
                let (file, bytes) = &self.files[index];
                (file, None, bytes.as_slice())
            });
            let _ = io.finish_trace();
            legacy_current(result)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(EvidenceError)
        }
    }
}
#[cfg(target_os = "linux")]
fn metadata(file: &std::fs::File, directory: bool, leaf: bool) -> Result<()> {
    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
    let m = file.metadata().map_err(|_| EvidenceError)?;
    require(
        m.uid() == 0
            && m.mode() & 0o022 == 0
            && if directory {
                m.is_dir()
            } else {
                m.is_file() && m.nlink() == 1
            },
    )?;
    if leaf {
        require(m.gid() == 10001 && m.mode() & 0o7777 == if directory { 0o750 } else { 0o440 })?;
    }
    for attr in [c"system.posix_acl_access", c"system.posix_acl_default"] {
        // SAFETY: valid held descriptor and static NUL-terminated name; query only.
        let result =
            unsafe { libc::fgetxattr(file.as_raw_fd(), attr.as_ptr(), std::ptr::null_mut(), 0) };
        require(
            result < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA),
        )?;
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn protected_child(parent: &std::fs::File, name: &str, directory: bool) -> Result<std::fs::File> {
    let io = CurrentIo::new(CurrentMode::Legacy);
    let result = legacy_current(protected_child_with(
        &io,
        parent,
        None,
        name,
        directory,
        CurrentRole::Current,
    ));
    let _ = io.finish_trace();
    result.map(CurrentFile::legacy)
}
#[cfg(target_os = "linux")]
fn protected_open(path: &Path, directory: bool) -> Result<std::fs::File> {
    let io = CurrentIo::new(CurrentMode::Legacy);
    let result = legacy_current(protected_open_with(&io, path, directory));
    let _ = io.finish_trace();
    result.map(CurrentFile::legacy)
}

const CURRENT_BINDING_DIRECTORY: &str = "/data/owner-beta-paper-evidence/current";
const CURRENT_BINDING_LEAF: &str = "current-binding.json";
const CURRENT_MAX_AGE_NS: i64 = 180_000_000_000;
const CURRENT_SNAPSHOT_ATTEMPTS: usize = 3;
const CURRENT_MAX_CLOCK_DIVERGENCE_NS: u64 = 1_000_000_000;

/// A canonical, schema-checked current envelope. Decoding this value never
/// grants protected-root or live-publisher authority.
pub struct UntrustedCurrentEnvelope {
    value: Value,
    bytes: Vec<u8>,
}

impl UntrustedCurrentEnvelope {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value = decode_canonical(bytes, SOURCE_LIMIT)?;
        validate_current_envelope(&value)?;
        Ok(Self {
            value,
            bytes: bytes.to_vec(),
        })
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn status(&self) -> Result<&str> {
        text(&self.value, "status")
    }

    fn is_current(&self) -> bool {
        self.value["status"] == "CURRENT"
    }
}

/// Parses and validates only untrusted bytes. Use `ProtectedCurrentBinding::open`
/// for the fixed-path protected read and live-fence check.
pub fn decode_current_envelope(bytes: &[u8]) -> Result<UntrustedCurrentEnvelope> {
    UntrustedCurrentEnvelope::decode(bytes)
}

fn current_schema() -> Result<Value> {
    serde_json::from_str(CURRENT_SCHEMA_JSON).map_err(|_| EvidenceError)
}

fn validate_current_envelope(value: &Value) -> Result<()> {
    let schema = current_schema()?;
    let source_schema: Value = serde_json::from_str(SCHEMA_JSON).map_err(|_| EvidenceError)?;
    for name in ["binding", "cohort", "process", "clock"] {
        require(schema["$defs"][name] == source_schema["$defs"][name])?;
    }

    // The existing small schema validator intentionally does not implement
    // JSON Schema conditionals. Validate the closed base properties here and
    // apply the frozen CURRENT/INVALID if/then/else rules explicitly below.
    let object = value.as_object().ok_or(EvidenceError)?;
    let required = schema["required"].as_array().ok_or(EvidenceError)?;
    require(object.len() == required.len())?;
    for key in required {
        let key = key.as_str().ok_or(EvidenceError)?;
        validate(
            object.get(key).ok_or(EvidenceError)?,
            &schema["properties"][key],
            &schema,
        )?;
    }

    let status = text(value, "status")?;
    let reason = text(value, "reason")?;
    let binding = &value["binding"];
    let binding_hash = &value["binding_sha256"];
    match status {
        "CURRENT" => {
            require(reason == "NONE" && !binding.is_null() && !binding_hash.is_null())?;
        }
        "INVALID" => {
            require(reason != "NONE")?;
            require(binding.is_null() == binding_hash.is_null())?;
        }
        _ => return Err(EvidenceError),
    }
    if !binding.is_null() {
        validate_binding_structure(binding, &schema)?;
        let full_hash = digest(&canonical(binding)?);
        require(binding_hash.as_str() == Some(full_hash.as_str()))?;
        let semantic = semantic_binding_hash(binding)?;
        require(binding["cohort"]["semantic_config_sha256"].as_str() == Some(&semantic))?;
    }
    Ok(())
}

fn semantic_binding_hash(binding: &Value) -> Result<String> {
    let mut material = binding.clone();
    let cohort = material
        .get_mut("cohort")
        .and_then(Value::as_object_mut)
        .ok_or(EvidenceError)?;
    require(cohort.remove("semantic_config_sha256").is_some())?;
    require(cohort.remove("cohort_sha256").is_some())?;
    Ok(digest(&canonical(&material)?))
}

fn clean_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && path[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn safe_identifier(value: &str) -> bool {
    (1..=63).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn validate_binding_structure(binding: &Value, schema: &Value) -> Result<()> {
    validate(binding, &schema["$defs"]["binding"], schema)?;
    validate(&binding["cohort"], &schema["$defs"]["cohort"], schema)?;

    for key in [
        "evidence_root",
        "release_root",
        "production_env_file",
        "calendar_dir",
        "calendar_file",
        "release_manifest_file",
        "data_root",
        "daily_config_file",
        "secret_root",
        "state_dir",
        "lock_file",
    ] {
        require(clean_absolute_path(text(binding, key)?))?;
    }
    for key in ["database", "project", "network"] {
        require(safe_identifier(text(binding, key)?))?;
    }

    let data_root = text(binding, "data_root")?;
    let evidence_root = text(binding, "evidence_root")?;
    let release_root = text(binding, "release_root")?;
    let cohort = &binding["cohort"];
    let verifier_revision = text(cohort, "verifier_revision")?;
    require(release_root.ends_with(&format!("/{verifier_revision}")))?;
    require(evidence_root == format!("{data_root}/owner-beta-paper-evidence"))?;
    require(
        text(binding, "daily_config_file")?
            == format!("{release_root}/.owner-beta-daily/config.json"),
    )?;
    require(
        text(binding, "release_manifest_file")?
            == format!("{release_root}/.lagrange-release-manifest"),
    )?;
    let public_directories = [
        format!("{evidence_root}/current"),
        format!("{evidence_root}/events"),
        format!("{data_root}/raw"),
        format!("{data_root}/owner-beta-daily/admitted"),
        format!("{data_root}/owner-beta-daily/historical-base/admitter"),
    ];
    let private_paths = [
        text(binding, "production_env_file")?,
        text(binding, "secret_root")?,
    ];
    for public in &public_directories {
        for private in private_paths {
            require(private != public && !private.starts_with(&format!("{public}/")))?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CurrentClock {
    realtime_ns: i64,
    monotonic_ns: i64,
    boottime_ns: i64,
    boot_id: String,
}

impl CurrentClock {
    fn from_envelope(value: &Value) -> Result<Self> {
        Ok(Self {
            realtime_ns: integer(&value["observed"], "realtime_ns")?,
            monotonic_ns: integer(&value["observed"], "monotonic_ns")?,
            boottime_ns: integer(&value["observed"], "boottime_ns")?,
            boot_id: text(value, "boot_id")?.to_owned(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrustedCurrentClock {
    realtime_ns: i64,
    monotonic_ns: i64,
    boottime_ns: i64,
    boot_id: String,
}

fn elapsed_clock_deltas(
    before: &TrustedCurrentClock,
    after: &TrustedCurrentClock,
) -> Result<[i64; 3]> {
    require(before.boot_id == after.boot_id)?;
    let deltas = [
        after
            .realtime_ns
            .checked_sub(before.realtime_ns)
            .ok_or(EvidenceError)?,
        after
            .boottime_ns
            .checked_sub(before.boottime_ns)
            .ok_or(EvidenceError)?,
        after
            .monotonic_ns
            .checked_sub(before.monotonic_ns)
            .ok_or(EvidenceError)?,
    ];
    require(deltas.iter().all(|delta| *delta >= 0))?;
    require(
        deltas[0].abs_diff(deltas[1]) <= CURRENT_MAX_CLOCK_DIVERGENCE_NS
            && deltas[1].abs_diff(deltas[2]) <= CURRENT_MAX_CLOCK_DIVERGENCE_NS,
    )?;
    Ok(deltas)
}

fn validate_current_timing(
    envelope: &UntrustedCurrentEnvelope,
    before: &TrustedCurrentClock,
    after: &TrustedCurrentClock,
) -> Result<()> {
    require(envelope.is_current())?;
    let observed = CurrentClock::from_envelope(&envelope.value)?;
    require(observed.boot_id == before.boot_id && observed.boot_id == after.boot_id)?;
    let _ = elapsed_clock_deltas(before, after)?;
    for current in [before, after] {
        validate_current_observation_age(&observed, current)?;
    }
    Ok(())
}

fn validate_current_observation_age(
    observed: &CurrentClock,
    current: &TrustedCurrentClock,
) -> Result<()> {
    require(observed.boot_id == current.boot_id)?;
    let ages = [
        current
            .realtime_ns
            .checked_sub(observed.realtime_ns)
            .ok_or(EvidenceError)?,
        current
            .boottime_ns
            .checked_sub(observed.boottime_ns)
            .ok_or(EvidenceError)?,
        current
            .monotonic_ns
            .checked_sub(observed.monotonic_ns)
            .ok_or(EvidenceError)?,
    ];
    require(ages.iter().all(|age| (0..CURRENT_MAX_AGE_NS).contains(age)))?;
    require(
        ages[0].abs_diff(ages[1]) <= CURRENT_MAX_CLOCK_DIVERGENCE_NS
            && ages[1].abs_diff(ages[2]) <= CURRENT_MAX_CLOCK_DIVERGENCE_NS,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StableCurrentKey {
    schema: String,
    boot_id: String,
    generation: String,
    binding_sha256: String,
    cohort: Value,
}

fn stable_current_key(envelope: &UntrustedCurrentEnvelope) -> Result<StableCurrentKey> {
    require(envelope.is_current())?;
    Ok(StableCurrentKey {
        schema: text(&envelope.value, "schema")?.to_owned(),
        boot_id: text(&envelope.value, "boot_id")?.to_owned(),
        generation: text(&envelope.value, "generation")?.to_owned(),
        binding_sha256: text(&envelope.value, "binding_sha256")?.to_owned(),
        cohort: envelope.value["binding"]["cohort"].clone(),
    })
}

fn validate_current_transition(
    previous: &UntrustedCurrentEnvelope,
    previous_bytes: &[u8],
    previous_clock: &TrustedCurrentClock,
    next: &UntrustedCurrentEnvelope,
    next_before: &TrustedCurrentClock,
    next_after: &TrustedCurrentClock,
) -> Result<()> {
    let previous_key = stable_current_key(previous)?;
    require(previous_key == stable_current_key(next)?)?;
    let _ = elapsed_clock_deltas(previous_clock, next_before)?;
    validate_current_timing(next, next_before, next_after)?;

    let previous_sequence = integer(&previous.value, "sequence")?;
    let next_sequence = integer(&next.value, "sequence")?;
    require(next_sequence >= previous_sequence)?;
    let previous_observed = CurrentClock::from_envelope(&previous.value)?;
    let next_observed = CurrentClock::from_envelope(&next.value)?;
    require(
        next_observed.realtime_ns >= previous_observed.realtime_ns
            && next_observed.monotonic_ns >= previous_observed.monotonic_ns
            && next_observed.boottime_ns >= previous_observed.boottime_ns,
    )?;
    if next_sequence == previous_sequence {
        require(previous_bytes == next.bytes)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_clock_value(clock_id: libc::clockid_t) -> Result<i64> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `value` is a valid writable timespec and clock_gettime only
    // samples the named kernel clock; it does not change host clock state.
    require(unsafe { libc::clock_gettime(clock_id, &mut value) } == 0)?;
    let seconds = i128::from(value.tv_sec);
    let nanoseconds = i128::from(value.tv_nsec);
    require(seconds >= 0 && (0..1_000_000_000).contains(&nanoseconds))?;
    let total = seconds
        .checked_mul(1_000_000_000)
        .and_then(|base| base.checked_add(nanoseconds))
        .ok_or(EvidenceError)?;
    i64::try_from(total).map_err(|_| EvidenceError)
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OfdLockObservation {
    lock_type: i16,
    whence: i16,
    start: i64,
    length: i64,
    pid: i32,
}

#[cfg(target_os = "linux")]
fn is_current_ofd_write_conflict(lock: OfdLockObservation) -> bool {
    lock.lock_type == libc::F_WRLCK as i16
        && lock.whence == libc::SEEK_SET as i16
        && lock.start == 0
        && lock.length == 0
        && lock.pid == -1
}

#[cfg(target_os = "linux")]
fn current_ofd_lock_observation(file: &std::fs::File) -> Result<OfdLockObservation> {
    use std::os::fd::AsRawFd;

    let mut lock = libc::flock {
        l_type: libc::F_RDLCK as i16,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    // SAFETY: `lock` is initialized for a read-only whole-file conflict query.
    // F_OFD_GETLK observes a conflicting live lock and never acquires or changes
    // a lock; `file` is a held read-only descriptor.
    require(unsafe { libc::fcntl(file.as_raw_fd(), libc::F_OFD_GETLK, &mut lock) } == 0)?;
    Ok(OfdLockObservation {
        lock_type: lock.l_type,
        whence: lock.l_whence,
        start: lock.l_start,
        length: lock.l_len,
        pid: lock.l_pid,
    })
}

#[cfg(target_os = "linux")]
fn require_live_current_ofd_writer(file: &std::fs::File) -> Result<()> {
    require(is_current_ofd_write_conflict(current_ofd_lock_observation(
        file,
    )?))
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CurrentFileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
    length: u64,
    ctime: i64,
    ctime_nsec: i64,
    mtime: i64,
    mtime_nsec: i64,
}

#[cfg(target_os = "linux")]
fn current_file_identity(file: &std::fs::File) -> Result<CurrentFileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| EvidenceError)?;
    Ok(CurrentFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        links: metadata.nlink(),
        length: metadata.len(),
        ctime: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
        mtime: metadata.mtime(),
        mtime_nsec: metadata.mtime_nsec(),
    })
}

#[cfg(target_os = "linux")]
fn validate_final_current_boundary(
    held: CurrentFileIdentity,
    initially_named: CurrentFileIdentity,
    finally_named: CurrentFileIdentity,
    lock: OfdLockObservation,
) -> Result<()> {
    require(held == initially_named && initially_named == finally_named)?;
    require(is_current_ofd_write_conflict(lock))
}

#[cfg(target_os = "linux")]
struct CurrentSnapshot {
    file: std::fs::File,
    envelope: UntrustedCurrentEnvelope,
    before: TrustedCurrentClock,
    after: TrustedCurrentClock,
}

#[cfg(target_os = "linux")]
fn read_current_snapshot_once() -> Result<CurrentSnapshot> {
    let io = CurrentIo::new(CurrentMode::Legacy);
    let result = legacy_current(read_current_snapshot_once_with(&io).outcome);
    let _ = io.finish_trace();
    result.map(|snapshot| CurrentSnapshot {
        file: snapshot.selected.expect("Legacy selected file").legacy(),
        envelope: snapshot.value.envelope,
        before: snapshot.value.before,
        after: snapshot.value.after,
    })
}

#[cfg(target_os = "linux")]
fn read_current_snapshot() -> Result<CurrentSnapshot> {
    let retried = retry_current_pass(
        || CurrentPass {
            outcome: read_current_snapshot_once().map_err(CurrentStop::Policy),
            settlement: None,
        },
        || EvidenceError,
    );
    legacy_current(retried.pass.outcome)
}

/// Read-only authority for the fixed current selector, scoped to one
/// evaluation. It has no public constructor, path override, clock override, or
/// deserialization path. A later generation requires a new evaluation.
pub struct ProtectedCurrentBinding {
    envelope: UntrustedCurrentEnvelope,
    _file: std::fs::File,
    last_clock: TrustedCurrentClock,
    active: bool,
}

impl ProtectedCurrentBinding {
    pub fn open() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let snapshot = read_current_snapshot()?;
            Ok(Self {
                envelope: snapshot.envelope,
                _file: snapshot.file,
                last_clock: snapshot.after,
                active: true,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(EvidenceError)
        }
    }

    /// Reopens the fixed selector and confirms the same stable key, monotone
    /// sequence/timestamps, a fresh clock window, and another live OFD fence.
    /// A failed revalidation permanently disables this instance.
    pub fn revalidate(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if !self.active {
                return Err(EvidenceError);
            }
            let result = read_current_snapshot().and_then(|snapshot| {
                validate_current_transition(
                    &self.envelope,
                    &self.envelope.bytes,
                    &self.last_clock,
                    &snapshot.envelope,
                    &snapshot.before,
                    &snapshot.after,
                )?;
                Ok(snapshot)
            });
            match result {
                Ok(snapshot) => {
                    self.envelope = snapshot.envelope;
                    self._file = snapshot.file;
                    self.last_clock = snapshot.after;
                    Ok(())
                }
                Err(_) => {
                    self.active = false;
                    Err(EvidenceError)
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.active = false;
            Err(EvidenceError)
        }
    }

    /// Returns the selector only after a fresh fixed-path revalidation.
    pub fn binding(&mut self) -> Result<&Value> {
        self.revalidate()?;
        self.envelope.value.get("binding").ok_or(EvidenceError)
    }

    /// Returns an explicitly untrusted copy after current authority is
    /// refreshed. This copy cannot construct a protected reader.
    pub fn snapshot(&mut self) -> Result<UntrustedCurrentEnvelope> {
        self.revalidate()?;
        Ok(UntrustedCurrentEnvelope {
            value: self.envelope.value.clone(),
            bytes: self.envelope.bytes.clone(),
        })
    }
}

// BEGIN C7 current checked custody

// Checked current custody uses the canonical market-data ledger. It does not
// authenticate a Raw binding or cover source-chain/config/SQLx resources.
type CurrentUnit = std::result::Result<(), crate::checked_close::Failure<std::convert::Infallible>>;
use crate::{
    checked_close::{
        Attempt as CurrentAttempt, Close as CurrentClose, Gap as CurrentGap, Owner as CurrentOwner,
        Report as CurrentReport, Role as CurrentRole,
    },
    raw_custody::RawInvocation,
};

#[derive(Clone, Copy)]
struct CurrentScope<'scope, 'attempt> {
    invocation: &'scope RawInvocation<'attempt>,
    expected: &'scope CurrentAttempt,
    captured: &'scope CurrentAttempt,
}
struct CurrentScopeCheck {
    expected: CurrentUnit,
    captured: CurrentUnit,
    owner: Option<CurrentUnit>,
}
impl CurrentScopeCheck {
    fn is_clean(&self) -> bool {
        self.expected.is_ok()
            && self.captured.is_ok()
            && self.owner.as_ref().is_none_or(std::result::Result::is_ok)
    }
    fn report(&self) -> Option<&CurrentReport> {
        self.expected
            .as_ref()
            .err()
            .or_else(|| self.captured.as_ref().err())
            .or_else(|| self.owner.as_ref().and_then(|r| r.as_ref().err()))
            .map(|f| f.report())
    }
}

/// A bounded diagnostic for one actual protected-context admission refusal.
/// Its constructors and stored native error remain private; the report is
/// observational and cannot be used to create another invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionGateFailureKind {
    Scope,
    Current,
    Reentrant,
}

#[expect(
    clippy::large_enum_variant,
    reason = "Whole original and per-call later errors stay owned through mandatory cleanup without new boxing or discarded diagnostics."
)]
enum ExecutionGateFailureSource {
    Scope {
        context_invocation: Option<crate::checked_close::Failure<std::convert::Infallible>>,
        expected_invocation: Option<crate::checked_close::Failure<std::convert::Infallible>>,
        current_scope: CurrentScopeCheck,
    },
    Current(CheckedCurrentFailure),
    Reentrant(crate::checked_close::Failure<std::convert::Infallible>),
}

/// Whole, lifetime-free admission failure retained by checked native callers.
/// It never converts a Current policy error into a StoreError or old report.
pub struct ExecutionGateFailure {
    source: Box<ExecutionGateFailureSource>,
}

impl ExecutionGateFailure {
    fn scope(
        context_invocation: Option<crate::checked_close::Failure<std::convert::Infallible>>,
        expected_invocation: Option<crate::checked_close::Failure<std::convert::Infallible>>,
        current_scope: CurrentScopeCheck,
    ) -> Self {
        Self {
            source: Box::new(ExecutionGateFailureSource::Scope {
                context_invocation,
                expected_invocation,
                current_scope,
            }),
        }
    }

    fn current(source: CheckedCurrentFailure) -> Self {
        Self {
            source: Box::new(ExecutionGateFailureSource::Current(source)),
        }
    }

    fn reentrant(source: crate::checked_close::Failure<std::convert::Infallible>) -> Self {
        Self {
            source: Box::new(ExecutionGateFailureSource::Reentrant(source)),
        }
    }

    pub fn kind(&self) -> ExecutionGateFailureKind {
        match self.source.as_ref() {
            ExecutionGateFailureSource::Scope { .. } => ExecutionGateFailureKind::Scope,
            ExecutionGateFailureSource::Current(_) => ExecutionGateFailureKind::Current,
            ExecutionGateFailureSource::Reentrant(_) => ExecutionGateFailureKind::Reentrant,
        }
    }

    pub fn current_failure(&self) -> Option<&CheckedCurrentFailure> {
        match self.source.as_ref() {
            ExecutionGateFailureSource::Current(source) => Some(source),
            _ => None,
        }
    }

    /// The actual first report, if the owning source operation had one.
    pub fn source_report(&self) -> Option<&crate::checked_close::Report> {
        match self.source.as_ref() {
            ExecutionGateFailureSource::Scope {
                context_invocation,
                expected_invocation,
                current_scope,
            } => context_invocation
                .as_ref()
                .map(crate::checked_close::Failure::report)
                .or_else(|| expected_invocation.as_ref().map(|failure| failure.report()))
                .or_else(|| current_scope.report()),
            ExecutionGateFailureSource::Current(source) => source.diagnostics().source_report(),
            ExecutionGateFailureSource::Reentrant(source) => Some(source.report()),
        }
    }
}

impl std::fmt::Debug for ExecutionGateFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionGateFailure")
            .field("kind", &self.kind())
            .field("source_report_present", &self.source_report().is_some())
            .finish_non_exhaustive()
    }
}

impl CurrentScope<'_, '_> {
    fn compare(&self, owner: Option<&CurrentAttempt>) -> CurrentScopeCheck {
        // No short-circuit, including after prior failure.
        let expected = self
            .invocation
            .settle_local_in(self.expected, Ok::<(), std::convert::Infallible>(()));
        let captured = self
            .invocation
            .settle_local_in(self.captured, Ok::<(), std::convert::Infallible>(()));
        let owner = owner.map(|attempt| {
            self.invocation
                .settle_local_in(attempt, Ok::<(), std::convert::Infallible>(()))
        });
        CurrentScopeCheck {
            expected,
            captured,
            owner,
        }
    }
}
struct CurrentCloseStep {
    role: CurrentRole,
    before: CurrentScopeCheck,
    after: CurrentScopeCheck,
}
fn finish_current_owner<R: CurrentClose>(
    scope: CurrentScope<'_, '_>,
    owner: CurrentOwner<R>,
    role: CurrentRole,
) -> CurrentCloseStep {
    let before = scope.compare(Some(owner.attempt()));
    // This bool describes aggregate ledger cleanliness, not native close success.
    let _ = owner.finish();
    let after = scope.compare(None);
    CurrentCloseStep {
        role,
        before,
        after,
    }
}
enum CurrentStop<E> {
    Policy(E),
    Scope(usize),
    Late,
}
struct CurrentPass<V, E> {
    outcome: std::result::Result<V, CurrentStop<E>>,
    settlement: Option<CurrentScopeCheck>,
}
struct CurrentRetry<E> {
    source: E,
    settlement: Option<CurrentScopeCheck>,
}
struct CurrentRetried<V, E> {
    pass: CurrentPass<V, E>,
    retries: Vec<CurrentRetry<E>>,
}
impl<V, E> std::fmt::Debug for CurrentRetried<V, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrentRetried")
            .field("value_present", &self.pass.outcome.is_ok())
            .field("retry_count", &self.retries.len())
            .finish_non_exhaustive()
    }
}
fn retry_current_pass<V, E>(
    mut read: impl FnMut() -> CurrentPass<V, E>,
    exhausted: impl FnOnce() -> E,
) -> CurrentRetried<V, E> {
    let mut retries = Vec::new();
    for _ in 0..CURRENT_SNAPSHOT_ATTEMPTS {
        let pass = read();
        match pass {
            CurrentPass {
                outcome: Err(CurrentStop::Policy(source)),
                settlement,
            } if settlement.as_ref().is_none_or(CurrentScopeCheck::is_clean) => {
                // A recoverable policy rejection is not yet a permanent ledger failure.
                retries.push(CurrentRetry { source, settlement });
            }
            pass => return CurrentRetried { pass, retries },
        }
    }
    CurrentRetried {
        pass: CurrentPass {
            outcome: Err(CurrentStop::Policy(exhausted())),
            settlement: None,
        },
        retries,
    }
}
#[derive(Default)]
struct CurrentTrace {
    checks: Vec<CurrentScopeCheck>,
    closes: Vec<CurrentCloseStep>,
    retries: Vec<CurrentRetry<EvidenceError>>,
}
impl CurrentTrace {
    fn append(&mut self, mut other: Self) {
        self.checks.append(&mut other.checks);
        self.closes.append(&mut other.closes);
        self.retries.append(&mut other.retries);
    }
    fn is_clean(&self) -> bool {
        self.checks.iter().all(CurrentScopeCheck::is_clean)
            && self
                .closes
                .iter()
                .all(|c| c.before.is_clean() && c.after.is_clean())
            && self.retries.iter().all(|r| {
                r.settlement
                    .as_ref()
                    .is_none_or(CurrentScopeCheck::is_clean)
            })
    }
}
#[derive(Clone, Copy)]
enum CurrentMode<'scope, 'attempt> {
    Legacy,
    Checked(CurrentScope<'scope, 'attempt>),
}
struct CurrentIo<'scope, 'attempt> {
    mode: CurrentMode<'scope, 'attempt>,
    trace: std::cell::RefCell<CurrentTrace>,
    armed: std::cell::Cell<bool>,
}
impl<'scope, 'attempt> CurrentIo<'scope, 'attempt> {
    fn new(mode: CurrentMode<'scope, 'attempt>) -> Self {
        Self {
            mode,
            trace: Default::default(),
            armed: std::cell::Cell::new(true),
        }
    }
    fn preflight(
        &self,
        owner: Option<&CurrentAttempt>,
    ) -> std::result::Result<(), CurrentStop<EvidenceError>> {
        if let CurrentMode::Checked(scope) = self.mode {
            let check = scope.compare(owner);
            let clean = check.is_clean();
            let mut trace = self.trace.borrow_mut();
            let index = trace.checks.len();
            trace.checks.push(check);
            if !clean {
                return Err(CurrentStop::Scope(index));
            }
        }
        Ok(())
    }
    fn settlement(&self) -> Option<CurrentScopeCheck> {
        match self.mode {
            CurrentMode::Legacy => None,
            CurrentMode::Checked(scope) => Some(scope.compare(None)),
        }
    }
    fn is_clean(&self) -> bool {
        match self.mode {
            CurrentMode::Legacy => true,
            CurrentMode::Checked(scope) => scope.captured.check() && self.trace.borrow().is_clean(),
        }
    }
    fn finish_trace(&self) -> CurrentTrace {
        self.armed.set(false);
        self.trace.take()
    }
    #[cfg(target_os = "linux")]
    fn adopt(&self, file: std::fs::File, role: CurrentRole) -> CurrentFile {
        match self.mode {
            CurrentMode::Legacy => CurrentFile::Legacy(file),
            CurrentMode::Checked(scope) => CurrentFile::Checked {
                owner: scope.captured.adopt(file, role),
                role,
            },
        }
    }
    #[cfg(target_os = "linux")]
    fn check_file(
        &self,
        file: &CurrentFile,
    ) -> std::result::Result<(), CurrentStop<EvidenceError>> {
        self.preflight(file.attempt())
    }
    #[cfg(target_os = "linux")]
    fn close(&self, file: CurrentFile) {
        match file {
            CurrentFile::Legacy(file) => drop(file),
            CurrentFile::Checked { owner, role } => {
                let CurrentMode::Checked(scope) = self.mode else {
                    unreachable!("checked owner only belongs to checked handling");
                };
                self.trace
                    .borrow_mut()
                    .closes
                    .push(finish_current_owner(scope, owner, role));
            }
        }
    }
}
impl Drop for CurrentIo<'_, '_> {
    fn drop(&mut self) {
        if self.armed.get()
            && let CurrentMode::Checked(scope) = self.mode
        {
            scope.captured.gap(CurrentGap::Abandoned);
        }
    }
}
#[cfg(target_os = "linux")]
enum CurrentFile {
    Legacy(std::fs::File),
    Checked {
        owner: CurrentOwner<std::fs::File>,
        role: CurrentRole,
    },
}
#[cfg(target_os = "linux")]
impl CurrentFile {
    fn file(&self) -> &std::fs::File {
        match self {
            Self::Legacy(file) => file,
            Self::Checked { owner, .. } => owner.get(),
        }
    }
    fn file_mut(&mut self) -> &mut std::fs::File {
        match self {
            Self::Legacy(file) => file,
            Self::Checked { owner, .. } => owner.get_mut(),
        }
    }
    fn attempt(&self) -> Option<&CurrentAttempt> {
        match self {
            Self::Legacy(_) => None,
            Self::Checked { owner, .. } => Some(owner.attempt()),
        }
    }
    fn legacy(self) -> std::fs::File {
        match self {
            Self::Legacy(file) => file,
            Self::Checked { .. } => unreachable!("private Legacy wrapper"),
        }
    }
    fn checked(self) -> CurrentOwner<std::fs::File> {
        match self {
            Self::Checked { owner, .. } => owner,
            Self::Legacy(_) => unreachable!("private checked constructor"),
        }
    }
}
#[cfg(target_os = "linux")]
fn current_policy<T>(outcome: Result<T>) -> std::result::Result<T, CurrentStop<EvidenceError>> {
    outcome.map_err(CurrentStop::Policy)
}
// The result is owned before the real close, including an original Err.
// Used at the native clone/boot retirement boundary and by ordinary models.
fn retire_current_read<V, E, C>(
    outcome: std::result::Result<V, E>,
    retire: impl FnOnce() -> C,
) -> (std::result::Result<V, E>, C) {
    let completion = retire();
    (outcome, completion)
}
fn replace_current_owner<R: CurrentClose>(
    scope: CurrentScope<'_, '_>,
    held: &mut Option<CurrentOwner<R>>,
    next: CurrentOwner<R>,
    active: &mut bool,
) -> Option<CurrentCloseStep> {
    held.replace(next).map(|old| {
        let step = finish_current_owner(scope, old, CurrentRole::Current);
        if !step.before.is_clean() || !step.after.is_clean() {
            *active = false;
        }
        step
    })
}
fn retain_current_transition<T, E>(
    result: std::result::Result<T, E>,
    active: &mut bool,
) -> std::result::Result<T, E> {
    if result.is_err() {
        *active = false;
    }
    result
}

/// Diagnostic classification only; never a binding or publication capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrentSourceKind {
    Policy,
    Scope,
}
/// Borrowed actual local scope results, without a Report reconstruction API.
pub struct CheckedCurrentScopeView<'a> {
    check: &'a CurrentScopeCheck,
}
impl<'a> CheckedCurrentScopeView<'a> {
    pub fn expected(&self) -> &'a CurrentUnit {
        &self.check.expected
    }
    pub fn captured(&self) -> &'a CurrentUnit {
        &self.check.captured
    }
    pub fn owner(&self) -> Option<&'a CurrentUnit> {
        self.check.owner.as_ref()
    }
}
/// An explicit close attempt and actual surrounding local results.
pub struct CheckedCurrentCloseView<'a> {
    step: &'a CurrentCloseStep,
}
impl<'a> CheckedCurrentCloseView<'a> {
    pub fn role(&self) -> CurrentRole {
        self.step.role
    }
    pub fn before(&self) -> CheckedCurrentScopeView<'a> {
        CheckedCurrentScopeView {
            check: &self.step.before,
        }
    }
    pub fn after(&self) -> CheckedCurrentScopeView<'a> {
        CheckedCurrentScopeView {
            check: &self.step.after,
        }
    }
}
struct CurrentValue {
    envelope: UntrustedCurrentEnvelope,
    before: TrustedCurrentClock,
    after: TrustedCurrentClock,
}
struct CurrentFailureData {
    source: Option<CurrentStop<EvidenceError>>,
    values: Vec<CurrentValue>,
    trace: CurrentTrace,
}
/// Owns the original source, returned values and separate actual later results.
pub struct CheckedCurrentFailure {
    data: Box<CurrentFailureData>,
}
/// Returned only after consuming the checked reader and explicitly draining it.
pub struct CheckedCurrentCompletion {
    values: Vec<CurrentValue>,
    trace: CurrentTrace,
}
pub struct CheckedCurrentFailureView<'a> {
    data: &'a CurrentFailureData,
}
pub struct CheckedCurrentCompletionView<'a> {
    completion: &'a CheckedCurrentCompletion,
}
impl CheckedCurrentFailure {
    pub fn diagnostics(&self) -> CheckedCurrentFailureView<'_> {
        CheckedCurrentFailureView { data: &self.data }
    }
}
impl<'a> CheckedCurrentFailureView<'a> {
    pub fn source_kind(&self) -> Option<CurrentSourceKind> {
        match &self.data.source {
            Some(CurrentStop::Policy(_)) => Some(CurrentSourceKind::Policy),
            Some(CurrentStop::Scope(_)) => Some(CurrentSourceKind::Scope),
            Some(CurrentStop::Late) | None => None,
        }
    }
    pub fn policy_source(&self) -> Option<&'a EvidenceError> {
        match &self.data.source {
            Some(CurrentStop::Policy(e)) => Some(e),
            _ => None,
        }
    }
    pub fn source_report(&self) -> Option<&'a CurrentReport> {
        match &self.data.source {
            Some(CurrentStop::Scope(i)) => self.data.trace.checks[*i].report(),
            _ => None,
        }
    }
    pub fn retained_values(&self) -> usize {
        self.data.values.len()
    }
    pub fn retry_count(&self) -> usize {
        self.data.trace.retries.len()
    }
    pub fn retry_source(&self, index: usize) -> Option<&'a EvidenceError> {
        self.data.trace.retries.get(index).map(|r| &r.source)
    }
    pub fn scope(&self, index: usize) -> Option<CheckedCurrentScopeView<'a>> {
        self.data
            .trace
            .checks
            .get(index)
            .map(|check| CheckedCurrentScopeView { check })
    }
    pub fn close(&self, index: usize) -> Option<CheckedCurrentCloseView<'a>> {
        self.data
            .trace
            .closes
            .get(index)
            .map(|step| CheckedCurrentCloseView { step })
    }
}
impl CheckedCurrentCompletion {
    pub fn diagnostics(&self) -> CheckedCurrentCompletionView<'_> {
        CheckedCurrentCompletionView { completion: self }
    }
    pub fn is_clean(&self) -> bool {
        self.trace.is_clean()
    }
}
impl<'a> CheckedCurrentCompletionView<'a> {
    pub fn retained_values(&self) -> usize {
        self.completion.values.len()
    }
    pub fn retained_snapshot(&self, index: usize) -> Option<&'a UntrustedCurrentEnvelope> {
        self.completion.values.get(index).map(|v| &v.envelope)
    }
    pub fn retry_count(&self) -> usize {
        self.completion.trace.retries.len()
    }
    pub fn retry_source(&self, index: usize) -> Option<&'a EvidenceError> {
        self.completion.trace.retries.get(index).map(|r| &r.source)
    }
    pub fn scope(&self, index: usize) -> Option<CheckedCurrentScopeView<'a>> {
        self.completion
            .trace
            .checks
            .get(index)
            .map(|check| CheckedCurrentScopeView { check })
    }
    pub fn close(&self, index: usize) -> Option<CheckedCurrentCloseView<'a>> {
        self.completion
            .trace
            .closes
            .get(index)
            .map(|step| CheckedCurrentCloseView { step })
    }
}
impl std::fmt::Debug for CheckedCurrentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let view = self.diagnostics();
        f.debug_struct("CheckedCurrentFailure")
            .field("source_kind", &view.source_kind())
            .field("retained_values", &view.retained_values())
            .field("retry_count", &view.retry_count())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for CheckedCurrentCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedCurrentCompletion")
            .field("retained_values", &self.values.len())
            .field("close_count", &self.trace.closes.len())
            .finish_non_exhaustive()
    }
}
fn current_failure(
    io: &CurrentIo<'_, '_>,
    source: Option<CurrentStop<EvidenceError>>,
    values: Vec<CurrentValue>,
) -> CheckedCurrentFailure {
    // The source is owned before making a policy rejection permanent.
    if source.is_some()
        && let CurrentMode::Checked(scope) = io.mode
        && scope.captured.check()
    {
        scope.captured.gap(CurrentGap::Abandoned);
    }
    CheckedCurrentFailure {
        data: Box::new(CurrentFailureData {
            source,
            values,
            trace: io.finish_trace(),
        }),
    }
}

#[cfg(target_os = "linux")]
fn protected_child_with(
    io: &CurrentIo<'_, '_>,
    parent: &std::fs::File,
    parent_attempt: Option<&CurrentAttempt>,
    name: &str,
    directory: bool,
    role: CurrentRole,
) -> std::result::Result<CurrentFile, CurrentStop<EvidenceError>> {
    use std::os::fd::{AsRawFd, FromRawFd};
    io.preflight(parent_attempt)?;
    let name = current_policy(std::ffi::CString::new(name).map_err(|_| EvidenceError))?;
    // SAFETY: live parent descriptor, valid NUL-terminated single component.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    current_policy(require(fd >= 0))?;
    // SAFETY: successful openat transfers this descriptor exactly once.
    let file = io.adopt(unsafe { std::fs::File::from_raw_fd(fd) }, role);
    let result: std::result::Result<_, CurrentStop<EvidenceError>> = (|| {
        io.check_file(&file)?;
        current_policy(metadata(file.file(), directory, true))
    })();
    match result {
        Ok(()) => Ok(file),
        Err(source) => {
            io.close(file);
            Err(source)
        }
    }
}
#[cfg(target_os = "linux")]
fn protected_open_with(
    io: &CurrentIo<'_, '_>,
    path: &Path,
    directory: bool,
) -> std::result::Result<CurrentFile, CurrentStop<EvidenceError>> {
    use std::os::fd::{AsRawFd, FromRawFd};
    io.preflight(None)?;
    let path = current_policy(path.to_str().ok_or(EvidenceError))?;
    current_policy(require(
        path.starts_with('/')
            && path != "/"
            && path[1..]
                .split('/')
                .all(|v| !v.is_empty() && v != "." && v != ".."),
    ))?;
    let root = current_policy(std::fs::File::open("/").map_err(|_| EvidenceError))?;
    let mut file = Some(io.adopt(root, CurrentRole::Ancestor));
    let result: std::result::Result<_, CurrentStop<EvidenceError>> = (|| {
        let parts: Vec<_> = path[1..].split('/').collect();
        for (index, part) in parts.iter().enumerate() {
            let is_dir = index < parts.len() - 1 || directory;
            let name = current_policy(std::ffi::CString::new(*part).map_err(|_| EvidenceError))?;
            io.check_file(file.as_ref().expect("traversal predecessor"))?;
            // SAFETY: held parent and validated component; never follow symlinks.
            let fd = unsafe {
                libc::openat(
                    file.as_ref()
                        .expect("traversal predecessor")
                        .file()
                        .as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW
                        | if is_dir { libc::O_DIRECTORY } else { 0 },
                )
            };
            current_policy(require(fd >= 0))?;
            // SAFETY: successful openat transfers ownership once.
            let child = io.adopt(
                unsafe { std::fs::File::from_raw_fd(fd) },
                CurrentRole::Ancestor,
            );
            handoff_current_handle(&mut file, child, |predecessor| io.close(predecessor));
            if !io.is_clean() {
                return Err(CurrentStop::Late);
            }
            let child = file.as_ref().expect("adopted child");
            io.check_file(child)?;
            current_policy(metadata(child.file(), is_dir, index == parts.len() - 1))?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(file.take().expect("final directory")),
        Err(source) => {
            if let Some(file) = file.take() {
                io.close(file);
            }
            Err(source)
        }
    }
}
#[cfg(target_os = "linux")]
fn legacy_current<T>(outcome: std::result::Result<T, CurrentStop<EvidenceError>>) -> Result<T> {
    match outcome {
        Ok(value) => Ok(value),
        Err(CurrentStop::Policy(source)) => Err(source),
        Err(CurrentStop::Scope(_) | CurrentStop::Late) => {
            unreachable!("Legacy handling has no checked ledger")
        }
    }
}
#[cfg(target_os = "linux")]
fn trusted_current_clock_with(
    io: &CurrentIo<'_, '_>,
) -> std::result::Result<TrustedCurrentClock, CurrentStop<EvidenceError>> {
    use std::io::Read;
    io.preflight(None)?;
    let realtime_ns = current_policy(linux_clock_value(libc::CLOCK_REALTIME))?;
    io.preflight(None)?;
    let monotonic_ns = current_policy(linux_clock_value(libc::CLOCK_MONOTONIC))?;
    io.preflight(None)?;
    let boottime_ns = current_policy(linux_clock_value(libc::CLOCK_BOOTTIME))?;
    io.preflight(None)?;
    let file = current_policy(
        std::fs::File::open("/proc/sys/kernel/random/boot_id").map_err(|_| EvidenceError),
    )?;
    let mut file = io.adopt(file, CurrentRole::BootId);
    let result: std::result::Result<_, CurrentStop<EvidenceError>> = (|| {
        io.check_file(&file)?;
        let mut bytes = String::new();
        current_policy(
            file.file_mut()
                .read_to_string(&mut bytes)
                .map_err(|_| EvidenceError),
        )?;
        Ok(bytes)
    })();
    let (result, ()) = retire_current_read(result, || io.close(file));
    let bytes = result?;
    if !io.is_clean() {
        return Err(CurrentStop::Late);
    }
    let boot_id = bytes.trim().to_owned();
    current_policy(require(format_ok(&boot_id, "uuid")))?;
    Ok(TrustedCurrentClock {
        realtime_ns,
        monotonic_ns,
        boottime_ns,
        boot_id,
    })
}
#[cfg(target_os = "linux")]
fn read_bounded_current_bytes_with(
    io: &CurrentIo<'_, '_>,
    file: &CurrentFile,
) -> std::result::Result<Vec<u8>, CurrentStop<EvidenceError>> {
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::fs::MetadataExt;

    io.check_file(file)?;
    let before = current_policy(file.file().metadata().map_err(|_| EvidenceError))?;
    current_policy(require(
        before.is_file() && before.len() > 0 && before.len() <= SOURCE_LIMIT as u64,
    ))?;
    io.check_file(file)?;
    let reader = current_policy(file.file().try_clone().map_err(|_| EvidenceError))?;
    let mut reader = io.adopt(reader, CurrentRole::Clone);
    let result: std::result::Result<_, CurrentStop<EvidenceError>> = (|| {
        io.check_file(&reader)?;
        current_policy(
            reader
                .file_mut()
                .seek(SeekFrom::Start(0))
                .map_err(|_| EvidenceError),
        )?;
        let mut bytes = Vec::with_capacity(before.len() as usize);
        io.check_file(&reader)?;
        current_policy(
            (&mut *reader.file_mut())
                .take(SOURCE_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| EvidenceError),
        )?;
        Ok(bytes)
    })();
    let (result, ()) = retire_current_read(result, || io.close(reader));
    let bytes = result?;
    if !io.is_clean() {
        return Err(CurrentStop::Late);
    }
    io.check_file(file)?;
    let after = current_policy(file.file().metadata().map_err(|_| EvidenceError))?;
    current_policy(require(
        bytes.len() as u64 == before.len()
            && (before.dev(), before.ino(), before.len())
                == (after.dev(), after.ino(), after.len()),
    ))?;
    Ok(bytes)
}

#[cfg(target_os = "linux")]
struct PendingCurrentSnapshot {
    selected: Option<CurrentFile>,
    value: CurrentValue,
}
fn handoff_current_handle<H>(held: &mut Option<H>, next: H, retire: impl FnOnce(H)) {
    if let Some(old) = held.replace(next) {
        retire(old);
    }
}
fn drain_current_handles<H, const N: usize>(files: &mut [Option<H>; N], mut retire: impl FnMut(H)) {
    for slot in files.iter_mut().rev() {
        if let Some(file) = slot.take() {
            retire(file);
        }
    }
}
#[cfg(target_os = "linux")]
fn read_current_snapshot_once_with(
    io: &CurrentIo<'_, '_>,
) -> CurrentPass<PendingCurrentSnapshot, EvidenceError> {
    let mut files: [Option<CurrentFile>; 6] = std::array::from_fn(|_| None);
    let outcome: std::result::Result<_, CurrentStop<EvidenceError>> = (|| {
        let before = trusted_current_clock_with(io)?;
        files[0] = Some(protected_open_with(
            io,
            Path::new(CURRENT_BINDING_DIRECTORY),
            true,
        )?);
        files[1] = Some(protected_child_with(
            io,
            files[0].as_ref().expect("parent").file(),
            files[0].as_ref().expect("parent").attempt(),
            CURRENT_BINDING_LEAF,
            false,
            CurrentRole::Current,
        )?);
        io.check_file(files[1].as_ref().expect("held"))?;
        current_policy(require_live_current_ofd_writer(
            files[1].as_ref().expect("held").file(),
        ))?;
        let bytes = read_bounded_current_bytes_with(io, files[1].as_ref().expect("held"))?;
        let envelope = current_policy(decode_current_envelope(&bytes))?;
        current_policy(require(envelope.is_current()))?;

        // Reopen the fixed directory and leaf independently. The path still has to
        // name the same validated inode and expose the same complete bytes.
        files[2] = Some(protected_open_with(
            io,
            Path::new(CURRENT_BINDING_DIRECTORY),
            true,
        )?);
        io.check_file(files[0].as_ref().expect("parent"))?;
        io.check_file(files[2].as_ref().expect("named_parent"))?;
        current_policy(require(
            current_file_identity(files[0].as_ref().expect("parent").file())
                .map_err(CurrentStop::Policy)?
                == current_file_identity(files[2].as_ref().expect("named_parent").file())
                    .map_err(CurrentStop::Policy)?,
        ))?;
        files[3] = Some(protected_child_with(
            io,
            files[2].as_ref().expect("named_parent").file(),
            files[2].as_ref().expect("named_parent").attempt(),
            CURRENT_BINDING_LEAF,
            false,
            CurrentRole::Current,
        )?);
        io.check_file(files[1].as_ref().expect("held"))?;
        io.check_file(files[3].as_ref().expect("named"))?;
        current_policy(require(
            current_policy(current_file_identity(
                files[1].as_ref().expect("held").file(),
            ))? == current_policy(current_file_identity(
                files[3].as_ref().expect("named").file(),
            ))?,
        ))?;
        current_policy(require_live_current_ofd_writer(
            files[3].as_ref().expect("named").file(),
        ))?;
        let named_bytes = read_bounded_current_bytes_with(io, files[3].as_ref().expect("named"))?;
        current_policy(require(bytes == named_bytes))?;

        let after = trusted_current_clock_with(io)?;
        current_policy(validate_current_timing(&envelope, &before, &after))?;

        // Close the read/revalidation gap: after the final complete bytes and clock
        // sample, resolve the compiled selector again and recheck its held identity
        // and live OFD fence. This is a final boundary observation, not an
        // atomicity claim across the filesystem and later database reads.
        files[4] = Some(protected_open_with(
            io,
            Path::new(CURRENT_BINDING_DIRECTORY),
            true,
        )?);
        io.check_file(files[2].as_ref().expect("named_parent"))?;
        io.check_file(files[4].as_ref().expect("final_parent"))?;
        current_policy(require(
            current_policy(current_file_identity(
                files[2].as_ref().expect("named_parent").file(),
            ))? == current_policy(current_file_identity(
                files[4].as_ref().expect("final_parent").file(),
            ))?,
        ))?;
        files[5] = Some(protected_child_with(
            io,
            files[4].as_ref().expect("final_parent").file(),
            files[4].as_ref().expect("final_parent").attempt(),
            CURRENT_BINDING_LEAF,
            false,
            CurrentRole::Current,
        )?);
        io.check_file(files[5].as_ref().expect("final_named"))?;
        let final_identity = current_policy(current_file_identity(
            files[5].as_ref().expect("final_named").file(),
        ))?;
        io.check_file(files[1].as_ref().expect("held"))?;
        io.check_file(files[3].as_ref().expect("named"))?;
        io.check_file(files[5].as_ref().expect("final_named"))?;
        current_policy(validate_final_current_boundary(
            current_policy(current_file_identity(
                files[1].as_ref().expect("held").file(),
            ))?,
            current_policy(current_file_identity(
                files[3].as_ref().expect("named").file(),
            ))?,
            final_identity,
            current_policy(current_ofd_lock_observation(
                files[5].as_ref().expect("final_named").file(),
            ))?,
        ))?;
        Ok(CurrentValue {
            envelope,
            before,
            after,
        })
    })();
    // A successful value and selected owner are outside any unit settlement T.
    let mut selected = if outcome.is_ok() {
        files[5].take()
    } else {
        None
    };
    drain_current_handles(&mut files, |file| io.close(file));
    if !io.is_clean()
        && let Some(file) = selected.take()
    {
        io.close(file);
    }
    let outcome = outcome.map(|value| PendingCurrentSnapshot { selected, value });
    CurrentPass {
        outcome,
        settlement: io.settlement(),
    }
}
#[cfg(target_os = "linux")]
fn read_checked_current_snapshot(
    io: &CurrentIo<'_, '_>,
) -> std::result::Result<PendingCurrentSnapshot, CurrentStop<EvidenceError>> {
    let retried = retry_current_pass(|| read_current_snapshot_once_with(io), || EvidenceError);
    io.trace.borrow_mut().retries.extend(retried.retries);
    if let Some(settlement) = retried.pass.settlement {
        io.trace.borrow_mut().checks.push(settlement);
    }
    retried.pass.outcome
}

/// The real fixed-selector reader borrowing the consumer's canonical scope.
/// Native handles and constructors remain private; this is not a Raw permit.
pub struct CheckedProtectedCurrentBinding<'scope, 'attempt> {
    scope: CurrentScope<'scope, 'attempt>,
    #[cfg(target_os = "linux")]
    held: Option<CurrentOwner<std::fs::File>>,
    #[cfg(target_os = "linux")]
    last_clock: TrustedCurrentClock,
    #[cfg(target_os = "linux")]
    active: bool,
    values: Vec<CurrentValue>,
    trace: CurrentTrace,
}
/// Non-Clone execution context created only by consuming a real checked
/// Current binding. It lends the same Attempt to native readers and keeps
/// the Current owner alive until every derived invocation borrow ends.
pub struct ProtectedExecutionContext<'scope, 'attempt> {
    current: std::cell::RefCell<CheckedProtectedCurrentBinding<'scope, 'attempt>>,
    scope: CurrentScope<'scope, 'attempt>,
    attempt: &'attempt CurrentAttempt,
    // Actual held owner token; every replacement is adopted by this SAME Attempt.
    held_attempt: Option<CurrentAttempt>,
}
impl ProtectedCurrentBinding {
    pub fn open_checked_in<'scope, 'attempt>(
        invocation: &'scope RawInvocation<'attempt>,
        expected: &'scope CurrentAttempt,
        captured: &'scope CurrentAttempt,
    ) -> std::result::Result<CheckedProtectedCurrentBinding<'scope, 'attempt>, CheckedCurrentFailure>
    {
        let scope = CurrentScope {
            invocation,
            expected,
            captured,
        };
        let io = CurrentIo::new(CurrentMode::Checked(scope));
        if let Err(source) = io.preflight(None) {
            return Err(current_failure(&io, Some(source), Vec::new()));
        }
        #[cfg(target_os = "linux")]
        {
            match read_checked_current_snapshot(&io) {
                Err(source) => Err(current_failure(&io, Some(source), Vec::new())),
                Ok(mut snapshot) => {
                    if !io.is_clean() {
                        if let Some(file) = snapshot.selected.take() {
                            io.close(file);
                        }
                        return Err(current_failure(&io, None, vec![snapshot.value]));
                    }
                    let held = snapshot
                        .selected
                        .take()
                        .expect("clean native selected owner")
                        .checked();
                    let last_clock = snapshot.value.after.clone();
                    Ok(CheckedProtectedCurrentBinding {
                        scope,
                        held: Some(held),
                        last_clock,
                        active: true,
                        values: vec![snapshot.value],
                        trace: io.finish_trace(),
                    })
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            scope.captured.gap(CurrentGap::NonLinux);
            let source = io.preflight(None).err();
            Err(current_failure(&io, source, Vec::new()))
        }
    }
}
impl<'scope, 'attempt> CheckedProtectedCurrentBinding<'scope, 'attempt> {
    fn held_attempt(&self) -> Option<&CurrentAttempt> {
        #[cfg(target_os = "linux")]
        {
            self.held.as_ref().map(CurrentOwner::attempt)
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    /// Transfers the actual checked Current owner into the only context that
    /// can lend a bound native invocation. No decoded value can call this.
    pub fn into_execution_context(self) -> ProtectedExecutionContext<'scope, 'attempt> {
        let scope = self.scope;
        let attempt = scope.invocation.attempt();
        let held_attempt = self.held_attempt().cloned();
        ProtectedExecutionContext {
            current: std::cell::RefCell::new(self),
            scope,
            attempt,
            held_attempt,
        }
    }

    pub fn revalidate(&mut self) -> std::result::Result<(), CheckedCurrentFailure> {
        let io = CurrentIo::new(CurrentMode::Checked(self.scope));
        #[cfg(target_os = "linux")]
        {
            if let Err(source) = io.preflight(self.held.as_ref().map(|owner| owner.attempt())) {
                self.active = false;
                return Err(current_failure(&io, Some(source), Vec::new()));
            }
            if !self.active {
                self.scope.captured.gap(CurrentGap::Reuse);
                let source = io
                    .preflight(self.held.as_ref().map(|owner| owner.attempt()))
                    .err();
                return Err(current_failure(&io, source, Vec::new()));
            }
            let mut snapshot = match read_checked_current_snapshot(&io) {
                Ok(snapshot) => snapshot,
                Err(source) => {
                    self.active = false;
                    return Err(current_failure(&io, Some(source), Vec::new()));
                }
            };
            if !io.is_clean() {
                self.active = false;
                if let Some(file) = snapshot.selected.take() {
                    io.close(file);
                }
                return Err(current_failure(&io, None, vec![snapshot.value]));
            }
            let previous = self.values.last().expect("actual initial snapshot");
            let transition = validate_current_transition(
                &previous.envelope,
                &previous.envelope.bytes,
                &self.last_clock,
                &snapshot.value.envelope,
                &snapshot.value.before,
                &snapshot.value.after,
            );
            let transition = retain_current_transition(transition, &mut self.active);
            if let Err(source) = transition {
                if let Some(file) = snapshot.selected.take() {
                    io.close(file);
                }
                return Err(current_failure(
                    &io,
                    Some(CurrentStop::Policy(source)),
                    vec![snapshot.value],
                ));
            }
            // Same original install boundary: envelope/new handle, old close,
            // then last clock. Ordinary prior/current values never enter unit T.
            self.values.push(snapshot.value);
            let next = snapshot
                .selected
                .take()
                .expect("validated native candidate")
                .checked();
            if let Some(step) =
                replace_current_owner(self.scope, &mut self.held, next, &mut self.active)
            {
                io.trace.borrow_mut().closes.push(step);
            }
            self.last_clock = self
                .values
                .last()
                .expect("installed snapshot")
                .after
                .clone();
            if !io.is_clean() {
                self.active = false;
                return Err(current_failure(&io, None, Vec::new()));
            }
            self.trace.append(io.finish_trace());
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let source = match io.preflight(None) {
                Err(source) => Some(source),
                Ok(()) => {
                    self.scope.captured.gap(CurrentGap::NonLinux);
                    io.preflight(None).err()
                }
            };
            Err(current_failure(&io, source, Vec::new()))
        }
    }
    pub fn binding(&mut self) -> std::result::Result<&Value, CheckedCurrentFailure> {
        self.revalidate()?;
        let result = self
            .values
            .last()
            .expect("current value")
            .envelope
            .value
            .get("binding")
            .ok_or(EvidenceError);
        match result {
            Ok(value) => Ok(value),
            Err(source) => {
                let io = CurrentIo::new(CurrentMode::Checked(self.scope));
                #[cfg(target_os = "linux")]
                {
                    self.active = false;
                }
                Err(current_failure(
                    &io,
                    Some(CurrentStop::Policy(source)),
                    Vec::new(),
                ))
            }
        }
    }
    pub fn snapshot(
        &mut self,
    ) -> std::result::Result<UntrustedCurrentEnvelope, CheckedCurrentFailure> {
        self.revalidate()?;
        let envelope = &self.values.last().expect("current value").envelope;
        Ok(UntrustedCurrentEnvelope {
            value: envelope.value.clone(),
            bytes: envelope.bytes.clone(),
        })
    }
    pub fn finish_local(mut self) -> CheckedCurrentCompletion {
        let io = CurrentIo::new(CurrentMode::Checked(self.scope));
        #[cfg(target_os = "linux")]
        if let Some(owner) = self.held.take() {
            io.trace.borrow_mut().closes.push(finish_current_owner(
                self.scope,
                owner,
                CurrentRole::Current,
            ));
        }
        io.trace.borrow_mut().checks.push(self.scope.compare(None));
        self.trace.append(io.finish_trace());
        CheckedCurrentCompletion {
            values: std::mem::take(&mut self.values),
            trace: std::mem::take(&mut self.trace),
        }
    }
}

// A borrowed ordinary cell and the actual Attempt share the production reentry seam.
// No native holder or authority can be constructed by this helper.
pub(crate) fn borrow_execution_current<'a, T>(
    current: &'a std::cell::RefCell<T>,
    attempt: &CurrentAttempt,
) -> std::result::Result<std::cell::RefMut<'a, T>, ExecutionGateFailure> {
    current.try_borrow_mut().map_err(|_| {
        attempt.gap(CurrentGap::Reuse);
        ExecutionGateFailure::reentrant(attempt.failure(None))
    })
}

impl ProtectedExecutionContext<'_, '_> {
    pub fn invocation(&self) -> RawInvocation<'_> {
        RawInvocation::with_native_context(self.attempt, self)
    }

    /// The callback cannot return a borrow into the private current binding.
    pub fn with_binding<T>(
        &self,
        f: impl FnOnce(&Value) -> T,
    ) -> std::result::Result<T, ExecutionGateFailure> {
        let invocation = self.invocation();
        if let Some(failure) = self.scope_failure(&invocation, self.scope.expected) {
            return Err(failure);
        }
        let mut current = borrow_execution_current(&self.current, self.attempt)?;
        // Complete the callback borrow before touching any other current state.
        current
            .binding()
            .map(f)
            .map_err(ExecutionGateFailure::current)
    }

    /// Gives one scoped callback the SAME genuine whole Current snapshot.
    /// The callback cannot retain a borrow into the privately held owner.
    pub fn with_snapshot<T>(
        &self,
        f: impl FnOnce(&UntrustedCurrentEnvelope) -> T,
    ) -> std::result::Result<T, ExecutionGateFailure> {
        let invocation = self.invocation();
        if let Some(failure) = self.scope_failure(&invocation, self.scope.expected) {
            return Err(failure);
        }
        let mut current = borrow_execution_current(&self.current, self.attempt)?;
        execution_snapshot_callback(current.snapshot(), f).map_err(ExecutionGateFailure::current)
    }

    /// Every per-call error has already moved into that call's lower carrier.
    pub fn finish_local(self) -> CheckedCurrentCompletion {
        self.current.into_inner().finish_local()
    }

    fn scope_failure(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &CurrentAttempt,
    ) -> Option<ExecutionGateFailure> {
        compare_execution_scopes(
            self.scope,
            self.attempt,
            self.held_attempt.as_ref(),
            invocation,
            expected,
        )
    }
}

fn compare_execution_scopes(
    scope: CurrentScope<'_, '_>,
    attempt: &CurrentAttempt,
    held_attempt: Option<&CurrentAttempt>,
    invocation: &RawInvocation<'_>,
    expected: &CurrentAttempt,
) -> Option<ExecutionGateFailure> {
    let actual = invocation.attempt();
    let context_invocation = if !actual.same_scope(attempt) {
        actual.gap(CurrentGap::ScopeMismatch);
        attempt.gap(CurrentGap::ScopeMismatch);
        Some(actual.failure(None))
    } else if actual.check() {
        None
    } else {
        Some(actual.failure(None))
    };
    let expected_invocation = invocation
        .settle_local_in(expected, Ok::<(), std::convert::Infallible>(()))
        .err();
    let current_scope = scope.compare(held_attempt);
    if context_invocation.is_some() || expected_invocation.is_some() || !current_scope.is_clean() {
        Some(ExecutionGateFailure::scope(
            context_invocation,
            expected_invocation,
            current_scope,
        ))
    } else {
        None
    }
}

impl crate::raw_custody::NativeAdmissionContext for ProtectedExecutionContext<'_, '_> {
    fn compare_scopes(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &CurrentAttempt,
    ) -> std::result::Result<(), ExecutionGateFailure> {
        match self.scope_failure(invocation, expected) {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    fn admit(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &CurrentAttempt,
    ) -> std::result::Result<(), ExecutionGateFailure> {
        if let Some(failure) = self.scope_failure(invocation, expected) {
            return Err(failure);
        }
        let mut current = borrow_execution_current(&self.current, self.attempt)?;
        current.revalidate().map_err(ExecutionGateFailure::current)
    }
}
impl Drop for CheckedProtectedCurrentBinding<'_, '_> {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if self.held.is_some() {
            // Gap only: the real canonical Owner field still drops once.
            self.scope.captured.gap(CurrentGap::Abandoned);
        }
    }
}
// END C7 current checked custody

// BEGIN C8 checked SourceChain custody
#[cfg(any(target_os = "linux", test))]
fn collect_source_names<N, E>(
    names: impl Iterator<Item = std::result::Result<N, E>>,
) -> std::result::Result<Vec<N>, E> {
    names.collect()
}
#[cfg(any(target_os = "linux", test))]
fn count_source_names<N, E>(names: impl Iterator<Item = std::result::Result<N, E>>) -> usize {
    names.count()
}
// A null native read fuses on both EOF and error. Copy bytes before next read.
#[cfg(any(target_os = "linux", test))]
fn next_source_name<E>(
    ended: &mut bool,
    mut read: impl FnMut() -> std::result::Result<Option<Vec<u8>>, E>,
) -> Option<std::result::Result<Vec<u8>, E>> {
    if *ended {
        return None;
    }
    loop {
        match read() {
            Ok(Some(name)) if name == b"." || name == b".." => {}
            Ok(Some(name)) => return Some(Ok(name)),
            Ok(None) => {
                *ended = true;
                return None;
            }
            Err(error) => {
                *ended = true;
                return Some(Err(error));
            }
        }
    }
}
#[cfg(any(target_os = "linux", test))]
fn retire_source_rows<R, V>(rows: Vec<(R, V)>, mut retire: impl FnMut(R)) -> Vec<V> {
    let mut values = Vec::with_capacity(rows.len());
    for (resource, value) in rows.into_iter().rev() {
        retire(resource);
        values.push(value);
    }
    values.reverse();
    values
}
fn check_source_owners<'a>(
    io: &CurrentIo<'_, '_>,
    owners: impl Iterator<Item = Option<&'a CurrentAttempt>>,
) -> std::result::Result<(), CurrentStop<EvidenceError>> {
    let mut first = io.preflight(None).err();
    for owner in owners {
        let result = io.preflight(owner);
        if first.is_none() {
            first = result.err();
        }
    }
    match first {
        Some(source) => Err(source),
        None => Ok(()),
    }
}
#[cfg(target_os = "linux")]
struct NativeDirectory {
    pointer: Option<std::ptr::NonNull<libc::DIR>>,
    attempt: CurrentAttempt,
}
#[cfg(target_os = "linux")]
impl CurrentClose for NativeDirectory {
    fn close(mut self) -> std::result::Result<(), crate::checked_close::Errno> {
        use crate::checked_close::Errno;
        let pointer = self.pointer.take().expect("private armed directory");
        // SAFETY: unique real opendir stream, disarmed before the one attempt.
        if unsafe { libc::closedir(pointer.as_ptr()) } == 0 {
            Ok(())
        } else {
            // Capture errno before any other effect, never retry a stale stream.
            match unsafe { *libc::__errno_location() } {
                libc::EINTR => Err(Errno::Interrupted),
                libc::EBADF => Err(Errno::BadDescriptor),
                libc::EIO => Err(Errno::Io),
                errno => Err(Errno::Other(errno)),
            }
        }
    }
}
#[cfg(target_os = "linux")]
impl Drop for NativeDirectory {
    fn drop(&mut self) {
        if let Some(pointer) = self.pointer.take() {
            self.attempt.gap(CurrentGap::Abandoned);
            // SAFETY: constructor/unwind fallback owns this still-armed stream.
            // No observed successful close is reported by this fallback.
            let _ = unsafe { libc::closedir(pointer.as_ptr()) };
        }
    }
}
#[cfg(target_os = "linux")]
fn enumerate_source<T>(
    io: &CurrentIo<'_, '_>,
    path: &Path,
    consume: impl FnOnce(
        &mut dyn Iterator<
            Item = std::result::Result<std::ffi::OsString, CurrentStop<EvidenceError>>,
        >,
    ) -> std::result::Result<T, CurrentStop<EvidenceError>>,
) -> std::result::Result<T, CurrentStop<EvidenceError>> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    io.preflight(None)?;
    match io.mode {
        CurrentMode::Legacy => {
            let mut names = current_policy(std::fs::read_dir(path).map_err(|_| EvidenceError))?
                .map(|entry| {
                    current_policy(
                        entry
                            .map(|entry| entry.file_name())
                            .map_err(|_| EvidenceError),
                    )
                });
            consume(&mut names)
        }
        CurrentMode::Checked(scope) => {
            let path = current_policy(
                std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| EvidenceError),
            )?;
            io.preflight(None)?;
            // SAFETY: valid same-path NUL terminated name. No fd ownership transfer.
            let pointer = unsafe { libc::opendir(path.as_ptr()) };
            let pointer = current_policy(std::ptr::NonNull::new(pointer).ok_or(EvidenceError))?;
            // Immediate adoption before any iteration/name allocation.
            let mut owner = scope.captured.adopt(
                NativeDirectory {
                    pointer: Some(pointer),
                    attempt: scope.captured.clone(),
                },
                CurrentRole::Enumeration,
            );
            let mut ended = false;
            let mut first_scope = None;
            let result = {
                let mut names = std::iter::from_fn(|| {
                    next_source_name(&mut ended, || {
                        if let Err(source) = io.preflight(Some(owner.attempt())) {
                            if let CurrentStop::Scope(index) = &source {
                                first_scope.get_or_insert(*index);
                            }
                            return Err(source);
                        }
                        let pointer = owner.get_mut().pointer.expect("live stream");
                        // SAFETY: uniquely borrowed stream; exact selected SDK types.
                        unsafe {
                            *libc::__errno_location() = 0;
                            #[cfg(not(target_env = "musl"))]
                            let entry = libc::readdir64(pointer.as_ptr());
                            #[cfg(target_env = "musl")]
                            let entry = libc::readdir(pointer.as_ptr());
                            if entry.is_null() {
                                let errno = *libc::__errno_location();
                                if errno == 0 {
                                    Ok(None)
                                } else {
                                    Err(CurrentStop::Policy(EvidenceError))
                                }
                            } else {
                                // Do not form a reference to a possibly variable-sized dirent.
                                let name =
                                    std::ffi::CStr::from_ptr((&raw const (*entry).d_name).cast());
                                Ok(Some(name.to_bytes().to_vec()))
                            }
                        }
                    })
                    .map(|name| name.map(std::ffi::OsString::from_vec))
                });
                consume(&mut names)
            };
            // Own the enumeration result before explicit close, even after Err.
            let (result, step) = retire_current_read(result, || {
                finish_current_owner(scope, owner, CurrentRole::Enumeration)
            });
            io.trace.borrow_mut().closes.push(step);
            match (result, first_scope) {
                (Ok(_), Some(index)) => Err(CurrentStop::Scope(index)),
                (result, _) => result,
            }
        }
    }
}
#[cfg(target_os = "linux")]
fn source_parent(
    io: &CurrentIo<'_, '_>,
    path: &Path,
) -> std::result::Result<CurrentFile, CurrentStop<EvidenceError>> {
    match io.mode {
        CurrentMode::Legacy => current_policy(protected_open(path, true)).map(CurrentFile::Legacy),
        CurrentMode::Checked(_) => protected_open_with(io, path, true),
    }
}
#[cfg(target_os = "linux")]
fn source_child(
    io: &CurrentIo<'_, '_>,
    parent: &CurrentFile,
    name: &str,
) -> std::result::Result<CurrentFile, CurrentStop<EvidenceError>> {
    match io.mode {
        CurrentMode::Legacy => {
            current_policy(protected_child(parent.file(), name, false)).map(CurrentFile::Legacy)
        }
        CurrentMode::Checked(_) => protected_child_with(
            io,
            parent.file(),
            parent.attempt(),
            name,
            false,
            CurrentRole::Source,
        ),
    }
}
#[cfg(target_os = "linux")]
struct SourceData {
    events: Vec<Value>,
    files: Vec<(CurrentFile, Vec<u8>)>,
    directory: std::path::PathBuf,
    partial: Vec<u8>,
    retained: Vec<Vec<u8>>,
}
#[derive(Default)]
struct SourceValues {
    events: Vec<Value>,
    bytes: Vec<Vec<u8>>,
    partial: Vec<u8>,
}
#[cfg(target_os = "linux")]
fn release_source_data(io: &CurrentIo<'_, '_>, data: SourceData) -> SourceValues {
    let SourceData {
        events,
        files,
        partial,
        retained,
        ..
    } = data;
    let mut bytes = retire_source_rows(files, |file| io.close(file));
    bytes.extend(retained);
    SourceValues {
        events,
        bytes,
        partial,
    }
}
#[cfg(target_os = "linux")]
fn revalidate_source_with<'a>(
    io: &CurrentIo<'_, '_>,
    directory: &Path,
    len: usize,
    held: impl Fn(usize) -> (&'a std::fs::File, Option<&'a CurrentAttempt>, &'a [u8]),
) -> std::result::Result<(), CurrentStop<EvidenceError>> {
    use std::{
        io::{Read, Seek, SeekFrom},
        os::unix::fs::MetadataExt,
    };
    check_source_owners(io, (0..len).map(|index| held(index).1))?;
    let mut parent = Some(source_parent(io, directory)?);
    let mut fresh = None;
    let mut clone = None;
    let result = (|| {
        io.check_file(parent.as_ref().expect("source parent"))?;
        let count = enumerate_source(io, directory, |names| Ok(count_source_names(names)))?;
        if !io.is_clean() {
            return Err(CurrentStop::Late);
        }
        current_policy(require(count == len))?;
        for index in 0..len {
            let (file, owner, bytes) = held(index);
            io.preflight(owner)?;
            let directory = parent.as_ref().expect("source parent");
            fresh = Some(source_child(io, directory, &format!("{index:06}.json"))?);
            io.preflight(owner)?;
            io.check_file(fresh.as_ref().expect("fresh source leaf"))?;
            let (a, b) = (
                current_policy(file.metadata().map_err(|_| EvidenceError))?,
                current_policy(
                    fresh
                        .as_ref()
                        .expect("fresh source leaf")
                        .file()
                        .metadata()
                        .map_err(|_| EvidenceError),
                )?,
            );
            current_policy(require(
                (a.dev(), a.ino(), a.len()) == (b.dev(), b.ino(), b.len()),
            ))?;
            io.preflight(owner)?;
            clone = Some(io.adopt(
                current_policy(file.try_clone().map_err(|_| EvidenceError))?,
                CurrentRole::Clone,
            ));
            let mut now = Vec::new();
            let read: std::result::Result<(), CurrentStop<EvidenceError>> = (|| {
                let reader = clone.as_mut().expect("source clone");
                io.check_file(reader)?;
                current_policy(
                    reader
                        .file_mut()
                        .seek(SeekFrom::Start(0))
                        .map_err(|_| EvidenceError),
                )?;
                io.check_file(reader)?;
                current_policy(
                    reader
                        .file_mut()
                        .take(SOURCE_LIMIT as u64 + 1)
                        .read_to_end(&mut now)
                        .map_err(|_| EvidenceError),
                )?;
                Ok(())
            })();
            let (read, ()) =
                retire_current_read(read, || io.close(clone.take().expect("source clone")));
            read?;
            if !io.is_clean() {
                return Err(CurrentStop::Late);
            }
            current_policy(require(now == bytes))?;
            io.close(fresh.take().expect("fresh source leaf"));
            if !io.is_clean() {
                return Err(CurrentStop::Late);
            }
        }
        Ok(())
    })();
    // Original result is already owned. Mandatory reverse cleanup even when dirty.
    drain_current_handles(&mut [parent.take(), fresh, clone], |file| io.close(file));
    result
}
#[cfg(target_os = "linux")]
fn open_source_with(
    io: &CurrentIo<'_, '_>,
    root: &Path,
    slot: &str,
) -> (
    std::result::Result<(), CurrentStop<EvidenceError>>,
    SourceData,
) {
    use std::io::Read;
    let mut data = SourceData {
        events: Vec::new(),
        files: Vec::new(),
        directory: root.join(&slot[7..]),
        partial: Vec::new(),
        retained: Vec::new(),
    };
    let mut parent = None;
    let mut leaf = None;
    let result = (|| {
        parent = Some(source_parent(io, &data.directory)?);
        io.check_file(parent.as_ref().expect("source parent"))?;
        let names = enumerate_source(io, &data.directory, |names| collect_source_names(names))?;
        if !io.is_clean() {
            return Err(CurrentStop::Late);
        }
        current_policy(require(!names.is_empty() && names.len() <= 64))?;
        let mut total = 0;
        for index in 0..names.len() {
            let name = format!("{index:06}.json");
            current_policy(require(names.iter().any(|v| v == name.as_str())))?;
            let directory = parent.as_ref().expect("source parent");
            leaf = Some(source_child(io, directory, &name)?);
            let file = leaf.as_mut().expect("source leaf");
            io.check_file(file)?;
            current_policy(
                file.file_mut()
                    .take(SOURCE_LIMIT as u64 + 1)
                    .read_to_end(&mut data.partial)
                    .map_err(|_| EvidenceError),
            )?;
            total += data.partial.len();
            current_policy(require(total <= SESSION_LIMIT))?;
            let value = current_policy(decode_canonical(&data.partial, SOURCE_LIMIT))?;
            current_policy(require(value["slot_id"] == slot))?;
            data.events.push(value);
            data.files.push((
                leaf.take().expect("accepted source leaf"),
                std::mem::take(&mut data.partial),
            ));
        }
        current_policy(verify_chain(&data.events, None))?;
        revalidate_source_with(io, &data.directory, data.files.len(), |index| {
            let (file, bytes) = &data.files[index];
            (file.file(), file.attempt(), bytes.as_slice())
        })
    })();
    if let Some(file) = leaf.take() {
        io.close(file);
    }
    if result.is_err() {
        // Parent was born before all accepted leaves.
        data.retained = retire_source_rows(std::mem::take(&mut data.files), |file| io.close(file));
    }
    if let Some(file) = parent.take() {
        io.close(file);
    }
    (result, data)
}
/// Diagnostic classification; never a Source or binding constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceChainSourceKind {
    Policy,
    Scope,
}
pub struct CheckedSourceFailure {
    data: Box<SourceFailureData>,
}
struct SourceFailureData {
    source: Option<CurrentStop<EvidenceError>>,
    values: SourceValues,
    trace: CurrentTrace,
}
pub struct CheckedSourceCompletion {
    values: SourceValues,
    trace: CurrentTrace,
}
pub struct CheckedSourceView<'a> {
    source: Option<&'a CurrentStop<EvidenceError>>,
    values: &'a SourceValues,
    trace: &'a CurrentTrace,
}
impl CheckedSourceFailure {
    pub fn diagnostics(&self) -> CheckedSourceView<'_> {
        CheckedSourceView {
            source: self.data.source.as_ref(),
            values: &self.data.values,
            trace: &self.data.trace,
        }
    }
}
impl CheckedSourceCompletion {
    pub fn diagnostics(&self) -> CheckedSourceView<'_> {
        CheckedSourceView {
            source: None,
            values: &self.values,
            trace: &self.trace,
        }
    }
    pub fn is_clean(&self) -> bool {
        self.trace.is_clean()
    }
}
impl<'a> CheckedSourceView<'a> {
    pub fn source_kind(&self) -> Option<SourceChainSourceKind> {
        match self.source {
            Some(CurrentStop::Policy(_)) => Some(SourceChainSourceKind::Policy),
            Some(CurrentStop::Scope(_)) => Some(SourceChainSourceKind::Scope),
            Some(CurrentStop::Late) | None => None,
        }
    }
    pub fn policy_source(&self) -> Option<&'a EvidenceError> {
        match self.source {
            Some(CurrentStop::Policy(source)) => Some(source),
            _ => None,
        }
    }
    pub fn source_report(&self) -> Option<&'a CurrentReport> {
        match self.source {
            Some(CurrentStop::Scope(index)) => self.trace.checks[*index].report(),
            _ => None,
        }
    }
    pub fn event_count(&self) -> usize {
        self.values.events.len()
    }
    pub fn retained_byte_bags(&self) -> usize {
        self.values.bytes.len()
    }
    pub fn partial_bytes(&self) -> usize {
        self.values.partial.len()
    }
    pub fn scope(&self, index: usize) -> Option<CheckedCurrentScopeView<'a>> {
        self.trace
            .checks
            .get(index)
            .map(|check| CheckedCurrentScopeView { check })
    }
    pub fn close(&self, index: usize) -> Option<CheckedCurrentCloseView<'a>> {
        self.trace
            .closes
            .get(index)
            .map(|step| CheckedCurrentCloseView { step })
    }
}
impl std::fmt::Debug for CheckedSourceView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckedSourceChain")
            .field("source_kind", &self.source_kind())
            .field("events", &self.event_count())
            .field("byte_bags", &self.retained_byte_bags())
            .field("close_steps", &self.trace.closes.len())
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for CheckedSourceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.diagnostics().fmt(f)
    }
}
impl std::fmt::Debug for CheckedSourceCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.diagnostics().fmt(f)
    }
}
/// A genuine held chain borrowing the caller's canonical scope.
pub struct CheckedProtectedSourceChain<'scope, 'attempt> {
    scope: CurrentScope<'scope, 'attempt>,
    #[cfg(target_os = "linux")]
    data: SourceData,
    active: std::cell::Cell<bool>,
    trace: std::cell::RefCell<CurrentTrace>,
}
fn source_failure(
    io: &CurrentIo<'_, '_>,
    source: Option<CurrentStop<EvidenceError>>,
    values: SourceValues,
) -> CheckedSourceFailure {
    if source.is_some()
        && let CurrentMode::Checked(scope) = io.mode
        && scope.captured.check()
    {
        scope.captured.gap(CurrentGap::Abandoned);
    }
    CheckedSourceFailure {
        data: Box::new(SourceFailureData {
            source,
            values,
            trace: io.finish_trace(),
        }),
    }
}
impl ProtectedSourceChain {
    pub fn open_checked_in<'scope, 'attempt>(
        root: &Path,
        slot: &str,
        invocation: &'scope RawInvocation<'attempt>,
        expected: &'scope CurrentAttempt,
        captured: &'scope CurrentAttempt,
    ) -> std::result::Result<CheckedProtectedSourceChain<'scope, 'attempt>, CheckedSourceFailure>
    {
        let scope = CurrentScope {
            invocation,
            expected,
            captured,
        };
        let io = CurrentIo::new(CurrentMode::Checked(scope));
        if let Err(source) = io.preflight(None) {
            return Err(source_failure(&io, Some(source), SourceValues::default()));
        }
        if let Err(source) = require(format_ok(slot, "sha256")) {
            return Err(source_failure(
                &io,
                Some(CurrentStop::Policy(source)),
                SourceValues::default(),
            ));
        }
        #[cfg(target_os = "linux")]
        {
            let (result, data) = open_source_with(&io, root, slot);
            let source = match result {
                Err(source) => Some(source),
                Ok(()) if !io.is_clean() => Some(CurrentStop::Late),
                Ok(()) => None,
            };
            if source.is_some() {
                let values = release_source_data(&io, data);
                return Err(source_failure(&io, source, values));
            }
            Ok(CheckedProtectedSourceChain {
                scope,
                data,
                active: std::cell::Cell::new(true),
                trace: std::cell::RefCell::new(io.finish_trace()),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = root;
            captured.gap(CurrentGap::NonLinux);
            let source = io.preflight(None).err();
            Err(source_failure(&io, source, SourceValues::default()))
        }
    }
}
impl CheckedProtectedSourceChain<'_, '_> {
    fn check_with(
        &self,
        io: &CurrentIo<'_, '_>,
    ) -> std::result::Result<(), CurrentStop<EvidenceError>> {
        #[cfg(target_os = "linux")]
        let result =
            check_source_owners(io, self.data.files.iter().map(|(file, _)| file.attempt()));
        #[cfg(not(target_os = "linux"))]
        let result = check_source_owners(io, std::iter::empty());
        result?;
        if !self.active.get() {
            self.scope.captured.gap(CurrentGap::Reuse);
            return io.preflight(None);
        }
        Ok(())
    }
    pub fn check_scope_in(
        &self,
        invocation: &RawInvocation<'_>,
        expected: &CurrentAttempt,
        captured: &CurrentAttempt,
    ) -> std::result::Result<(), CheckedSourceFailure> {
        let io = CurrentIo::new(CurrentMode::Checked(self.scope));
        // Compare supplied scope to all genuine stored endpoints BEFORE inspection.
        let supplied = CurrentIo::new(CurrentMode::Checked(CurrentScope {
            invocation,
            expected,
            captured,
        }));
        let a = supplied.preflight(Some(self.scope.expected));
        let b = supplied.preflight(Some(self.scope.captured));
        let c = self.check_with(&supplied);
        let own = self.check_with(&io);
        let source = a.err().or_else(|| b.err()).or_else(|| c.err());
        if let Some(source) = source {
            self.active.set(false);
            // The source index belongs to supplied's genuine trace.
            self.trace.borrow_mut().append(io.finish_trace());
            return Err(source_failure(
                &supplied,
                Some(source),
                SourceValues::default(),
            ));
        }
        self.trace.borrow_mut().append(supplied.finish_trace());
        if let Err(source) = own {
            self.active.set(false);
            return Err(source_failure(&io, Some(source), SourceValues::default()));
        }
        self.trace.borrow_mut().append(io.finish_trace());
        Ok(())
    }
    fn checked_view(&self) -> std::result::Result<(), CheckedSourceFailure> {
        let io = CurrentIo::new(CurrentMode::Checked(self.scope));
        if let Err(source) = self.check_with(&io) {
            self.active.set(false);
            return Err(source_failure(&io, Some(source), SourceValues::default()));
        }
        self.trace.borrow_mut().append(io.finish_trace());
        Ok(())
    }
    pub fn events(&self) -> std::result::Result<&[Value], CheckedSourceFailure> {
        self.checked_view()?;
        #[cfg(target_os = "linux")]
        {
            Ok(&self.data.events)
        }
        #[cfg(not(target_os = "linux"))]
        {
            unreachable!("nonLinux construction refused")
        }
    }
    pub fn first(&self) -> std::result::Result<&Value, CheckedSourceFailure> {
        Ok(&self.events()?[0])
    }
    pub fn optional_event(
        &self,
        kind: &str,
    ) -> std::result::Result<Option<&Value>, CheckedSourceFailure> {
        Ok(self.events()?.iter().find(|event| event["kind"] == kind))
    }
    pub fn event(&self, kind: &str) -> std::result::Result<&Value, CheckedSourceFailure> {
        if let Some(value) = self.optional_event(kind)? {
            return Ok(value);
        }
        self.active.set(false);
        let io = CurrentIo::new(CurrentMode::Checked(self.scope));
        Err(source_failure(
            &io,
            Some(CurrentStop::Policy(EvidenceError)),
            SourceValues::default(),
        ))
    }
    pub fn revalidate(&self) -> std::result::Result<(), CheckedSourceFailure> {
        let io = CurrentIo::new(CurrentMode::Checked(self.scope));
        let result = self.check_with(&io);
        #[cfg(target_os = "linux")]
        let result = result.and_then(|()| {
            revalidate_source_with(&io, &self.data.directory, self.data.files.len(), |index| {
                let (file, bytes) = &self.data.files[index];
                (file.file(), file.attempt(), bytes.as_slice())
            })
        });
        let source = match result {
            Err(source) => Some(source),
            Ok(()) if !io.is_clean() => Some(CurrentStop::Late),
            Ok(()) => None,
        };
        if source.is_some() {
            self.active.set(false);
            return Err(source_failure(&io, source, SourceValues::default()));
        }
        self.trace.borrow_mut().append(io.finish_trace());
        Ok(())
    }
    pub fn finish_local(mut self) -> CheckedSourceCompletion {
        let io = CurrentIo::new(CurrentMode::Checked(self.scope));
        #[cfg(target_os = "linux")]
        let values = release_source_data(
            &io,
            SourceData {
                events: std::mem::take(&mut self.data.events),
                files: std::mem::take(&mut self.data.files),
                directory: std::mem::take(&mut self.data.directory),
                partial: std::mem::take(&mut self.data.partial),
                retained: std::mem::take(&mut self.data.retained),
            },
        );
        #[cfg(not(target_os = "linux"))]
        let values = SourceValues::default();
        io.trace.borrow_mut().checks.push(self.scope.compare(None));
        let mut trace = self.trace.take();
        trace.append(io.finish_trace());
        self.active.set(false);
        CheckedSourceCompletion { values, trace }
    }
}
impl Drop for CheckedProtectedSourceChain<'_, '_> {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        let held = !self.data.files.is_empty();
        #[cfg(not(target_os = "linux"))]
        let held = false;
        if self.active.get() || held {
            self.scope.captured.gap(CurrentGap::Abandoned);
        }
        // Genuine Owner fields still perform their once-only fallback.
    }
}
// END C8 checked SourceChain custody

#[cfg(test)]
mod current_binding_tests {
    use super::*;

    const VALID: &[u8] = include_bytes!(
        "../../../tests/fixtures/owner_beta_paper_current_binding/valid-current.json"
    );
    const REFRESH: &[u8] = include_bytes!(
        "../../../tests/fixtures/owner_beta_paper_current_binding/valid-refresh.json"
    );
    const INVALID_STARTING: &[u8] = include_bytes!(
        "../../../tests/fixtures/owner_beta_paper_current_binding/valid-invalid-starting.json"
    );
    const INVALID_STOPPED: &[u8] = include_bytes!(
        "../../../tests/fixtures/owner_beta_paper_current_binding/valid-invalid-stopped.json"
    );
    const REJECTIONS: &[u8] =
        include_bytes!("../../../tests/fixtures/owner_beta_paper_current_binding/rejections.json");
    const CURRENT_SCHEMA_DOC: &str =
        include_str!("../../../docs/contracts/owner-beta-paper-current-binding-v1.schema.json");
    const AMENDMENT1_LAYOUT_VECTORS: &str = include_str!(
        "../../../tests/fixtures/owner_beta_paper_current_binding/compatibility-vectors.json"
    );

    fn valid() -> UntrustedCurrentEnvelope {
        decode_current_envelope(VALID).expect("synthetic shared CURRENT envelope")
    }

    fn clock_at(envelope: &UntrustedCurrentEnvelope, offset: i64) -> TrustedCurrentClock {
        let observed = CurrentClock::from_envelope(&envelope.value).expect("fixture observed");
        TrustedCurrentClock {
            realtime_ns: observed.realtime_ns + offset,
            monotonic_ns: observed.monotonic_ns + offset,
            boottime_ns: observed.boottime_ns + offset,
            boot_id: observed.boot_id,
        }
    }

    fn clock_at_ages(
        envelope: &UntrustedCurrentEnvelope,
        ages_ns: [i64; 3],
    ) -> TrustedCurrentClock {
        let observed = CurrentClock::from_envelope(&envelope.value).expect("fixture observed");
        TrustedCurrentClock {
            realtime_ns: observed.realtime_ns + ages_ns[0],
            boottime_ns: observed.boottime_ns + ages_ns[1],
            monotonic_ns: observed.monotonic_ns + ages_ns[2],
            boot_id: observed.boot_id,
        }
    }

    // SYNTHETIC regression model of the pre-CR1 timing check. It checked each
    // clock age and within-read deltas independently, but omitted the
    // publication-observed-to-trusted age divergence.
    fn legacy_timing_without_publication_age_divergence(
        envelope: &UntrustedCurrentEnvelope,
        before: &TrustedCurrentClock,
        after: &TrustedCurrentClock,
    ) -> bool {
        if !envelope.is_current() {
            return false;
        }
        let Ok(observed) = CurrentClock::from_envelope(&envelope.value) else {
            return false;
        };
        if observed.boot_id != before.boot_id
            || observed.boot_id != after.boot_id
            || elapsed_clock_deltas(before, after).is_err()
        {
            return false;
        }
        [before, after].into_iter().all(|current| {
            [
                (current.realtime_ns, observed.realtime_ns),
                (current.boottime_ns, observed.boottime_ns),
                (current.monotonic_ns, observed.monotonic_ns),
            ]
            .into_iter()
            .all(|(now, seen)| {
                now.checked_sub(seen)
                    .is_some_and(|age| (0..CURRENT_MAX_AGE_NS).contains(&age))
            })
        })
    }

    fn mutate_pointer(value: &mut Value, pointer: &str, replacement: Value) {
        let (parent, key) = pointer.rsplit_once('/').expect("fixture pointer");
        let parent = value.pointer_mut(parent).expect("fixture parent");
        parent
            .as_object_mut()
            .expect("object parent")
            .insert(key.to_owned(), replacement);
    }

    #[test]
    fn shared_synthetic_envelopes_are_untrusted_and_status_separated() {
        let current = decode_current_envelope(VALID).expect("synthetic valid CURRENT");
        let refresh = decode_current_envelope(REFRESH).expect("synthetic valid refresh");
        let starting = decode_current_envelope(INVALID_STARTING).expect("synthetic INVALID");
        let stopped = decode_current_envelope(INVALID_STOPPED).expect("synthetic INVALID");
        assert_eq!(current.status().expect("status"), "CURRENT");
        assert_eq!(refresh.status().expect("status"), "CURRENT");
        assert_eq!(starting.status().expect("status"), "INVALID");
        assert_eq!(stopped.status().expect("status"), "INVALID");
        assert!(!starting.is_current() && !stopped.is_current());
        assert!(stable_current_key(&starting).is_err());
        assert!(stable_current_key(&stopped).is_err());
        assert_eq!(
            stable_current_key(&current).expect("stable key"),
            stable_current_key(&refresh).expect("same key across refresh")
        );
    }

    #[test]
    fn current_schema_matches_frozen_document_and_copied_source_definitions() {
        let current: Value = serde_json::from_str(CURRENT_SCHEMA_JSON).expect("embedded schema");
        let documented: Value = serde_json::from_str(CURRENT_SCHEMA_DOC).expect("frozen schema");
        let source: Value = serde_json::from_str(SCHEMA_JSON).expect("source schema");
        assert_eq!(current, documented);
        for key in ["binding", "cohort", "process", "clock"] {
            assert_eq!(current["$defs"][key], source["$defs"][key]);
        }
        assert!(current["allOf"][0].get("if").is_some());
        assert!(current["allOf"][0].get("then").is_some());
        assert!(current["allOf"][0].get("else").is_some());
    }

    #[test]
    fn amendment1_synthetic_layout_vectors_match_actual_public_mount_boundary() {
        let vectors: Value = serde_json::from_str(AMENDMENT1_LAYOUT_VECTORS)
            .expect("parent-provided compatibility vectors");
        assert_eq!(vectors["synthetic"], true);
        assert_eq!(vectors["production_eligible"], false);
        for case in vectors["cases"].as_array().expect("vector cases") {
            let bytes = canonical(&case["envelope"]).expect("canonical synthetic envelope");
            let accepted = decode_current_envelope(&bytes).is_ok();
            assert_eq!(
                accepted,
                case["expected_after_amendment1"]
                    .as_bool()
                    .expect("expected bool"),
                "layout vector {}",
                case["name"].as_str().expect("vector name")
            );
        }
    }

    #[test]
    fn all_shared_synthetic_rejection_mutations_are_rejected() {
        let mutations: Value = serde_json::from_slice(REJECTIONS).expect("shared mutations");
        for mutation in mutations.as_array().expect("mutation list") {
            let mut value: Value = serde_json::from_slice(VALID).expect("valid fixture");
            let path = mutation["path"].as_str().expect("mutation path");
            mutate_pointer(&mut value, path, mutation["value"].clone());
            if path == "/binding" {
                value["binding_sha256"] = if value["binding"].is_null() {
                    Value::Null
                } else {
                    json!(digest(
                        &canonical(&value["binding"]).expect("binding bytes")
                    ))
                };
            } else if path.starts_with("/binding/") {
                value["binding_sha256"] = json!(digest(
                    &canonical(&value["binding"]).expect("mutated binding bytes")
                ));
            }
            let bytes = canonical(&value).expect("canonical mutation");
            assert!(
                decode_current_envelope(&bytes).is_err(),
                "accepted shared mutation: {}",
                mutation["name"]
            );
        }
    }

    #[test]
    fn canonical_and_resource_limits_fail_closed() {
        assert!(decode_current_envelope(&[b' '; SOURCE_LIMIT + 1]).is_err());
        let nested = format!(
            "{}0{}",
            "[".repeat(MAX_DEPTH + 1),
            "]".repeat(MAX_DEPTH + 1)
        );
        assert!(decode_current_envelope(nested.as_bytes()).is_err());
        let needle = b",\"schema\":\"owner-beta-paper-current-binding-v1\"";
        let position = VALID
            .windows(needle.len())
            .position(|window| window == needle)
            .expect("root schema field");
        let mut duplicate = VALID[..position].to_vec();
        duplicate.extend_from_slice(
            b",\"schema\":\"owner-beta-paper-current-binding-v1\",\"schema\":\"owner-beta-paper-current-binding-v1\"",
        );
        duplicate.extend_from_slice(&VALID[position + needle.len()..]);
        assert!(serde_json::from_slice::<Value>(&duplicate).is_ok());
        assert!(decode_current_envelope(&duplicate).is_err());
        let float = String::from_utf8(VALID.to_vec())
            .expect("fixture UTF-8")
            .replacen("\"sequence\":1", "\"sequence\":1.0", 1);
        assert!(decode_current_envelope(float.as_bytes()).is_err());
    }

    #[test]
    fn current_freshness_rejects_future_stale_rollback_and_clock_divergence() {
        let envelope = valid();
        let observed = CurrentClock::from_envelope(&envelope.value).expect("observed");
        let at_observation = clock_at(&envelope, 0);
        assert!(validate_current_timing(&envelope, &at_observation, &at_observation).is_ok());

        let exactly_one_second_divergence = TrustedCurrentClock {
            realtime_ns: observed.realtime_ns + CURRENT_MAX_CLOCK_DIVERGENCE_NS as i64,
            ..at_observation.clone()
        };
        assert!(
            validate_current_timing(&envelope, &at_observation, &exactly_one_second_divergence)
                .is_ok()
        );

        let future = TrustedCurrentClock {
            realtime_ns: observed.realtime_ns - 1,
            ..at_observation.clone()
        };
        assert!(validate_current_timing(&envelope, &future, &at_observation).is_err());

        let stale = TrustedCurrentClock {
            realtime_ns: observed.realtime_ns + CURRENT_MAX_AGE_NS,
            monotonic_ns: observed.monotonic_ns + CURRENT_MAX_AGE_NS,
            boottime_ns: observed.boottime_ns + CURRENT_MAX_AGE_NS,
            ..at_observation.clone()
        };
        assert!(validate_current_timing(&envelope, &stale, &stale).is_err());

        let rollback = TrustedCurrentClock {
            realtime_ns: observed.realtime_ns - 1,
            ..at_observation.clone()
        };
        assert!(validate_current_timing(&envelope, &at_observation, &rollback).is_err());

        let divergent = TrustedCurrentClock {
            realtime_ns: observed.realtime_ns + 1_000_000_001,
            ..at_observation.clone()
        };
        assert!(validate_current_timing(&envelope, &at_observation, &divergent).is_err());

        let wrong_boot = TrustedCurrentClock {
            boot_id: "00000000-0000-4000-8000-000000000004".to_owned(),
            ..at_observation.clone()
        };
        assert!(validate_current_timing(&envelope, &wrong_boot, &wrong_boot).is_err());
    }

    #[test]
    fn observed_to_trusted_clock_ages_reject_wall_step_suspend_and_root_trace() {
        let envelope = valid();
        let second = 1_000_000_000_i64;
        for (name, before_ages, after_ages) in [
            (
                "wall step",
                [5 * second, second, second],
                [
                    5 * second + 100_000_000,
                    second + 100_000_000,
                    second + 100_000_000,
                ],
            ),
            (
                "suspend",
                [5 * second, 5 * second, second],
                [
                    5 * second + 100_000_000,
                    5 * second + 100_000_000,
                    second + 100_000_000,
                ],
            ),
            (
                "reviewer root trace",
                [40 * second, 10 * second, 10 * second],
                [
                    40 * second + 100_000_000,
                    10 * second + 100_000_000,
                    10 * second + 100_000_000,
                ],
            ),
        ] {
            let before = clock_at_ages(&envelope, before_ages);
            let after = clock_at_ages(&envelope, after_ages);
            assert!(
                legacy_timing_without_publication_age_divergence(&envelope, &before, &after),
                "counterexample no longer reaches the pre-CR1 check: {name}"
            );
            assert!(
                validate_current_timing(&envelope, &before, &after).is_err(),
                "accepted publication age divergence: {name}"
            );
        }

        let exact = clock_at_ages(&envelope, [second, 0, 0]);
        assert!(validate_current_timing(&envelope, &exact, &exact).is_ok());
        let over = clock_at_ages(&envelope, [second + 1, 0, 0]);
        assert!(validate_current_timing(&envelope, &over, &over).is_err());
    }

    #[test]
    fn refreshed_snapshot_requires_same_key_and_monotone_sequence_and_times() {
        let previous = decode_current_envelope(VALID).expect("previous");
        let next = decode_current_envelope(REFRESH).expect("refresh");
        let previous_bytes = previous.bytes.clone();
        let previous_clock = clock_at(&previous, 100);
        let next_before = clock_at(&next, 100);
        let next_after = clock_at(&next, 200);
        assert!(
            validate_current_transition(
                &previous,
                &previous_bytes,
                &previous_clock,
                &next,
                &next_before,
                &next_after
            )
            .is_ok()
        );

        let mut changed_generation_value: Value = serde_json::from_slice(REFRESH).expect("JSON");
        changed_generation_value["generation"] = json!("33333333-3333-4333-8333-333333333333");
        let changed_generation =
            decode_current_envelope(&canonical(&changed_generation_value).expect("canonical"))
                .expect("structurally valid generation change");
        assert!(
            validate_current_transition(
                &previous,
                &previous_bytes,
                &previous_clock,
                &changed_generation,
                &next_before,
                &next_after
            )
            .is_err()
        );

        let mut changed_boot_value: Value = serde_json::from_slice(REFRESH).expect("JSON");
        changed_boot_value["boot_id"] = json!("00000000-0000-4000-8000-000000000004");
        let changed_boot =
            decode_current_envelope(&canonical(&changed_boot_value).expect("canonical"))
                .expect("structurally valid boot change");
        assert!(
            validate_current_transition(
                &previous,
                &previous_bytes,
                &previous_clock,
                &changed_boot,
                &next_before,
                &next_after
            )
            .is_err()
        );

        let rolled_back_clock = TrustedCurrentClock {
            realtime_ns: previous_clock.realtime_ns - 1,
            ..next_before.clone()
        };
        assert!(
            validate_current_transition(
                &previous,
                &previous_bytes,
                &previous_clock,
                &next,
                &rolled_back_clock,
                &next_after
            )
            .is_err()
        );

        let mut changed_sequence_value: Value = serde_json::from_slice(REFRESH).expect("JSON");
        changed_sequence_value["sequence"] = json!(0);
        let changed_sequence =
            decode_current_envelope(&canonical(&changed_sequence_value).expect("canonical"))
                .expect("structurally valid sequence");
        assert!(
            validate_current_transition(
                &previous,
                &previous_bytes,
                &previous_clock,
                &changed_sequence,
                &next_before,
                &next_after
            )
            .is_err()
        );

        let mut same_sequence_value: Value = serde_json::from_slice(VALID).expect("JSON");
        same_sequence_value["observed"]["realtime_ns"] = json!(
            same_sequence_value["observed"]["realtime_ns"]
                .as_i64()
                .unwrap()
                + 1
        );
        same_sequence_value["observed"]["monotonic_ns"] = json!(
            same_sequence_value["observed"]["monotonic_ns"]
                .as_i64()
                .unwrap()
                + 1
        );
        same_sequence_value["observed"]["boottime_ns"] = json!(
            same_sequence_value["observed"]["boottime_ns"]
                .as_i64()
                .unwrap()
                + 1
        );
        let same_sequence =
            decode_current_envelope(&canonical(&same_sequence_value).expect("canonical"))
                .expect("structurally valid same sequence refresh");
        assert!(
            validate_current_transition(
                &previous,
                &previous_bytes,
                &previous_clock,
                &same_sequence,
                &next_before,
                &next_after
            )
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_whole_file_ofd_writer_conflict_is_accepted() {
        let accepted = OfdLockObservation {
            lock_type: libc::F_WRLCK as i16,
            whence: libc::SEEK_SET as i16,
            start: 0,
            length: 0,
            pid: -1,
        };
        assert!(is_current_ofd_write_conflict(accepted));
        assert!(!is_current_ofd_write_conflict(OfdLockObservation {
            lock_type: libc::F_UNLCK as i16,
            ..accepted
        }));
        assert!(!is_current_ofd_write_conflict(OfdLockObservation {
            pid: 123,
            ..accepted
        }));
        assert!(!is_current_ofd_write_conflict(OfdLockObservation {
            start: 1,
            ..accepted
        }));
        assert!(!is_current_ofd_write_conflict(OfdLockObservation {
            length: 1,
            ..accepted
        }));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn final_fixed_path_boundary_requires_same_metadata_and_live_ofd_fence() {
        let identity = CurrentFileIdentity {
            device: 1,
            inode: 2,
            mode: 0o100440,
            uid: 10001,
            gid: 10001,
            links: 1,
            length: 128,
            ctime: 3,
            ctime_nsec: 4,
            mtime: 5,
            mtime_nsec: 6,
        };
        let live = OfdLockObservation {
            lock_type: libc::F_WRLCK as i16,
            whence: libc::SEEK_SET as i16,
            start: 0,
            length: 0,
            pid: -1,
        };
        assert!(validate_final_current_boundary(identity, identity, identity, live).is_ok());
        assert!(
            validate_final_current_boundary(
                identity,
                identity,
                CurrentFileIdentity {
                    inode: 9,
                    ..identity
                },
                live,
            )
            .is_err()
        );
        assert!(
            validate_final_current_boundary(
                identity,
                identity,
                CurrentFileIdentity {
                    mode: 0o100400,
                    ..identity
                },
                live,
            )
            .is_err()
        );
        assert!(
            validate_final_current_boundary(
                identity,
                identity,
                identity,
                OfdLockObservation {
                    lock_type: libc::F_UNLCK as i16,
                    ..live
                },
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod checked_current_tests {
    use super::*;
    use crate::checked_close::{Errno, Gap, Issue, Role};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    struct Counted {
        name: &'static str,
        events: Rc<RefCell<Vec<&'static str>>>,
        result: std::result::Result<(), Errno>,
    }
    impl CurrentClose for Counted {
        fn close(self) -> std::result::Result<(), Errno> {
            self.events.borrow_mut().push(self.name);
            self.result
        }
    }
    fn counted(
        name: &'static str,
        events: &Rc<RefCell<Vec<&'static str>>>,
        result: std::result::Result<(), Errno>,
    ) -> Counted {
        Counted {
            name,
            events: Rc::clone(events),
            result,
        }
    }
    struct Body {
        bytes: Box<str>,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for Body {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    fn body(drops: &Rc<Cell<usize>>, text: &str) -> Body {
        Body {
            bytes: text.into(),
            drops: Rc::clone(drops),
        }
    }

    #[test]
    fn checked_current_policy_error_precedes_unknown_close() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(counted("outer", &events, Ok(())), Role::Source);
        let file = attempt.adopt(
            counted("file", &events, Err(Errno::Interrupted)),
            Role::Current,
        );
        let drops = Rc::new(Cell::new(0));
        let error = body(&drops, "private original policy");
        let address = error.bytes.as_ptr();
        let (original, close) = retire_current_read(Err::<(), _>(error), || {
            finish_current_owner(scope, file, Role::Current)
        });
        let error = match original {
            Err(error) => error,
            Ok(()) => panic!("original error"),
        };
        assert_eq!(error.bytes.as_ptr(), address);
        assert!(close.before.is_clean());
        assert!(!close.after.is_clean());
        assert!(
            close
                .after
                .expected
                .as_ref()
                .err()
                .unwrap()
                .body()
                .is_none()
        );
        assert_eq!(attempt.report().unknown(), 1);
        assert_eq!(attempt.report().live(), 1);
        assert_eq!(&*events.borrow(), &["file"]);
        assert_eq!(drops.get(), 0);
        drop(error);
        assert_eq!(drops.get(), 1);
        let _ = outer.finish();
        assert_eq!(&*events.borrow(), &["file", "outer"]);
    }

    #[test]
    fn checked_current_traversal_adopts_child_then_closes_predecessor() {
        for close_result in [Ok(()), Err(Errno::Io)] {
            let attempt = CurrentAttempt::default();
            let invocation = RawInvocation::new(&attempt);
            let scope = CurrentScope {
                invocation: &invocation,
                expected: &attempt,
                captured: &attempt,
            };
            let events = Rc::new(RefCell::new(Vec::new()));
            let mut held = Some(attempt.adopt(
                counted("predecessor", &events, close_result),
                Role::Ancestor,
            ));
            let child = attempt.adopt(counted("child", &events, Ok(())), Role::Ancestor);
            assert_eq!(attempt.report().live(), 2);
            handoff_current_handle(&mut held, child, |old| {
                let _ = finish_current_owner(scope, old, Role::Ancestor);
            });
            assert_eq!(attempt.report().live(), 1);
            let validation = scope.compare(held.as_ref().map(|o| o.attempt()));
            if validation.is_clean() {
                events.borrow_mut().push("validate");
            }
            let mut slots = [held];
            drain_current_handles(&mut slots, |owner| {
                let _ = finish_current_owner(scope, owner, Role::Ancestor);
            });
            assert!(slots[0].is_none());
            assert_eq!(attempt.report().attempted(), 2);
            if close_result.is_ok() {
                assert_eq!(&*events.borrow(), &["predecessor", "validate", "child"]);
            } else {
                assert_eq!(&*events.borrow(), &["predecessor", "child"]);
            }
        }
    }

    #[test]
    fn checked_current_clone_finishes_before_after_metadata() {
        for close_result in [Ok(()), Err(Errno::BadDescriptor)] {
            let attempt = CurrentAttempt::default();
            let invocation = RawInvocation::new(&attempt);
            let scope = CurrentScope {
                invocation: &invocation,
                expected: &attempt,
                captured: &attempt,
            };
            let events = Rc::new(RefCell::new(Vec::new()));
            let clone = attempt.adopt(counted("clone", &events, close_result), Role::Clone);
            let drops = Rc::new(Cell::new(0));
            let read = body(&drops, "ordinary read bytes");
            let address = read.bytes.as_ptr();
            let (read, close) = retire_current_read(Ok::<_, Body>(read), || {
                finish_current_owner(scope, clone, Role::Clone)
            });
            let read = match read {
                Ok(read) => read,
                Err(_) => panic!("read succeeded"),
            };
            assert_eq!(read.bytes.as_ptr(), address);
            if close.after.is_clean() {
                events.borrow_mut().push("after_metadata");
            }
            assert_eq!(events.borrow().first(), Some(&"clone"));
            assert_eq!(
                events.borrow().len(),
                if close_result.is_ok() { 2 } else { 1 }
            );
            assert_eq!(drops.get(), 0);
            drop(read);
            assert_eq!(drops.get(), 1);
        }
    }

    #[test]
    fn checked_current_retry_policy_preserves_clean_recovery() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let calls = Cell::new(0);
        let drops = Rc::new(Cell::new(0));
        let recovered = retry_current_pass(
            || {
                calls.set(calls.get() + 1);
                CurrentPass {
                    outcome: if calls.get() == 1 {
                        Err(CurrentStop::Policy(body(&drops, "transient")))
                    } else {
                        Ok(19)
                    },
                    settlement: Some(scope.compare(None)),
                }
            },
            || body(&drops, "exhausted"),
        );
        assert_eq!(calls.get(), 2);
        assert!(matches!(recovered.pass.outcome, Ok(19)));
        assert_eq!(recovered.retries.len(), 1);
        assert!(attempt.check());
        assert_eq!(drops.get(), 0);
        drop(recovered);
        assert_eq!(drops.get(), 1);

        let calls = Cell::new(0);
        let exhausted = retry_current_pass(
            || {
                calls.set(calls.get() + 1);
                CurrentPass::<(), _> {
                    outcome: Err(CurrentStop::Policy(body(&drops, "retry"))),
                    settlement: Some(scope.compare(None)),
                }
            },
            || body(&drops, "exhausted"),
        );
        assert_eq!(calls.get(), CURRENT_SNAPSHOT_ATTEMPTS);
        assert_eq!(exhausted.retries.len(), CURRENT_SNAPSHOT_ATTEMPTS);
        match &exhausted.pass.outcome {
            Err(CurrentStop::Policy(e)) => assert_eq!(&*e.bytes, "exhausted"),
            _ => panic!("terminal original policy"),
        }
        assert!(attempt.check());
        drop(exhausted);

        let events = Rc::new(RefCell::new(Vec::new()));
        let calls = Cell::new(0);
        let stopped = retry_current_pass(
            || {
                calls.set(calls.get() + 1);
                let owner =
                    attempt.adopt(counted("uncertain", &events, Err(Errno::Io)), Role::Current);
                let close = finish_current_owner(scope, owner, Role::Current);
                CurrentPass::<(), _> {
                    outcome: Err(CurrentStop::Policy(body(&drops, "earlier error"))),
                    settlement: Some(close.after),
                }
            },
            || body(&drops, "must not exhaust"),
        );
        assert_eq!(calls.get(), 1);
        assert!(stopped.retries.is_empty());
        assert_eq!(&*events.borrow(), &["uncertain"]);
        assert!(!attempt.check());
    }

    #[test]
    fn checked_current_pending_value_survives_late_cleanup_failure() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut selected = Some(attempt.adopt(counted("selected", &events, Ok(())), Role::Current));
        let mut temporary = [Some(attempt.adopt(
            counted("temporary", &events, Err(Errno::Io)),
            Role::Ancestor,
        ))];
        let drops = Rc::new(Cell::new(0));
        let original = Ok::<_, Body>(body(&drops, "actual ordinary successful value"));
        drain_current_handles(&mut temporary, |owner| {
            let _ = finish_current_owner(scope, owner, Role::Ancestor);
        });
        if !scope.compare(None).is_clean() {
            let _ = finish_current_owner(scope, selected.take().unwrap(), Role::Current);
        }
        let pending = CurrentPass {
            outcome: original.map_err(CurrentStop::Policy),
            settlement: Some(scope.compare(None)),
        };
        assert!(pending.outcome.is_ok());
        assert!(!pending.settlement.as_ref().unwrap().is_clean());
        assert_eq!(drops.get(), 0);
        assert_eq!(&*events.borrow(), &["temporary", "selected"]);
        assert_eq!(attempt.report().live(), 0);
        let terminal = attempt.finish(Ok::<(), std::convert::Infallible>(()));
        assert!(terminal.as_ref().err().unwrap().body().is_none());
        assert!(pending.outcome.is_ok());
        drop(pending);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn checked_current_replacement_retains_previous_and_candidate() {
        // Rejected transition: candidate drains, old remains for explicit finish.
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let old = attempt.adopt(counted("old", &events, Ok(())), Role::Current);
        let rejected = attempt.adopt(counted("rejected", &events, Ok(())), Role::Current);
        let mut active = true;
        let drops = Rc::new(Cell::new(0));
        let transition =
            retain_current_transition(Err::<(), _>(body(&drops, "transition")), &mut active);
        assert!(transition.is_err());
        assert!(!active);
        let _ = finish_current_owner(scope, rejected, Role::Current);
        assert_eq!(attempt.report().live(), 1);
        assert_eq!(&*events.borrow(), &["rejected"]);
        let _ = finish_current_owner(scope, old, Role::Current);
        assert_eq!(&*events.borrow(), &["rejected", "old"]);
        drop(transition);
        assert_eq!(drops.get(), 1);

        // Separate valid transition: install new handle then unknown old close.
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut held = Some(attempt.adopt(counted("old", &events, Err(Errno::Io)), Role::Current));
        let candidate = attempt.adopt(counted("candidate", &events, Ok(())), Role::Current);
        let previous_value = body(&drops, "previous");
        let candidate_value = body(&drops, "candidate");
        let mut active = true;
        assert!(retain_current_transition(Ok::<(), Body>(()), &mut active).is_ok());
        let step = replace_current_owner(scope, &mut held, candidate, &mut active).unwrap();
        assert!(!step.after.is_clean());
        assert!(!active);
        assert_eq!(&*events.borrow(), &["old"]);
        assert_eq!(&*previous_value.bytes, "previous");
        assert_eq!(&*candidate_value.bytes, "candidate");
        let _ = finish_current_owner(scope, held.take().unwrap(), Role::Current);
        assert_eq!(&*events.borrow(), &["old", "candidate"]);
        assert_eq!(attempt.report().attempted(), 2);
        drop((previous_value, candidate_value));
        assert_eq!(drops.get(), 3);
    }

    #[test]
    fn checked_current_both_foreign_comparisons_run_when_dirty() {
        let actual = CurrentAttempt::default();
        let expected = CurrentAttempt::default();
        let captured = CurrentAttempt::default();
        let foreign_owner = CurrentAttempt::default();
        actual.gap(Gap::TerminalPublication);
        let invocation = RawInvocation::new(&actual);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &expected,
            captured: &captured,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let owner = foreign_owner.adopt(counted("foreign", &events, Ok(())), Role::Current);
        let admission = scope.compare(Some(owner.attempt()));
        assert!(!admission.is_clean());
        for attempt in [&expected, &captured, &foreign_owner] {
            assert_eq!(
                attempt.report().first(),
                Some(Issue::Gap(Gap::ScopeMismatch))
            );
        }
        assert_eq!(
            actual.report().first(),
            Some(Issue::Gap(Gap::TerminalPublication))
        );
        let refusal =
            match ProtectedCurrentBinding::open_checked_in(&invocation, &expected, &captured) {
                Err(failure) => failure,
                Ok(_) => panic!("foreign scopes must refuse before native effects"),
            };
        assert_eq!(
            refusal.diagnostics().source_kind(),
            Some(CurrentSourceKind::Scope)
        );
        assert!(refusal.diagnostics().close(0).is_none());
        assert!(refusal.diagnostics().source_report().is_some());
        let step = finish_current_owner(scope, owner, Role::Current);
        assert!(!step.before.is_clean());
        assert!(!step.after.is_clean());
        assert_eq!(foreign_owner.report().attempted(), 1);
        assert_eq!(actual.report().attempted(), 0);
        assert_eq!(expected.report().attempted(), 0);
        assert_eq!(captured.report().attempted(), 0);
        assert_eq!(&*events.borrow(), &["foreign"]);
    }

    #[test]
    fn checked_current_drop_unwind_and_reuse_are_fail_closed() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(counted("outer", &events, Ok(())), Role::Source);
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _owned = attempt.adopt(counted("inner", &events, Ok(())), Role::Current);
            let _io = CurrentIo::new(CurrentMode::Checked(scope));
            panic!("ordinary model unwind");
        }));
        assert!(unwind.is_err());
        assert_eq!(&*events.borrow(), &["inner"]);
        assert_eq!(attempt.report().first(), Some(Issue::Gap(Gap::Abandoned)));
        assert_eq!(attempt.report().live(), 1);
        assert!(!scope.compare(None).is_clean());
        assert_eq!(attempt.report().attempted(), 1);
        let _ = outer.finish();
        assert_eq!(&*events.borrow(), &["inner", "outer"]);
    }

    #[test]
    fn checked_current_error_debug_requires_no_body_traits() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let drops = Rc::new(Cell::new(0));
        let original = body(&drops, "SECRET-ORDINARY-BODY");
        let address = original.bytes.as_ptr();
        let failure = match invocation.settle_local_in(&attempt, Err::<(), _>(original)) {
            Err(failure) => failure,
            Ok(()) => panic!("actual source"),
        };
        let retained = CurrentRetried {
            pass: CurrentPass::<(), _> {
                outcome: Err(CurrentStop::Policy(failure)),
                settlement: None,
            },
            retries: Vec::new(),
        };
        let debug = format!("{retained:?}");
        assert!(!debug.contains("SECRET"));
        assert!(!debug.contains("ORDINARY"));
        match &retained.pass.outcome {
            Err(CurrentStop::Policy(failure)) => {
                assert_eq!(failure.body().unwrap().bytes.as_ptr(), address)
            }
            _ => panic!("whole actual failure"),
        }
        assert_eq!(drops.get(), 0);
        drop(retained);
        assert_eq!(drops.get(), 1);
    }
}

#[cfg(test)]
mod checked_source_tests {
    use super::*;
    use crate::checked_close::{Close, Errno, Issue};
    use std::{
        cell::{Cell, RefCell},
        convert::Infallible,
        rc::Rc,
    };

    struct Counted {
        name: &'static str,
        log: Rc<RefCell<Vec<&'static str>>>,
        error: Option<Errno>,
    }
    impl Close for Counted {
        fn close(self) -> std::result::Result<(), Errno> {
            self.log.borrow_mut().push(self.name);
            self.error.map_or(Ok(()), Err)
        }
    }
    struct Row(Box<str>);
    struct Body(Box<str>);
    fn counted(
        log: &Rc<RefCell<Vec<&'static str>>>,
        name: &'static str,
        error: Option<Errno>,
    ) -> Counted {
        Counted {
            name,
            log: log.clone(),
            error,
        }
    }

    #[test]
    fn source_chain_first_error_survives_enumeration_unknown_close() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let log = Rc::new(RefCell::new(Vec::new()));
        let owner = attempt.adopt(
            counted(&log, "enumeration", Some(Errno::Interrupted)),
            CurrentRole::Enumeration,
        );
        let body = Body("original ordinary policy".into());
        let pointer = body.0.as_ptr();
        let (outcome, step) = retire_current_read(Err::<(), _>(body), || {
            finish_current_owner(scope, owner, CurrentRole::Enumeration)
        });
        assert_eq!(outcome.as_ref().err().unwrap().0.as_ptr(), pointer);
        assert!(step.before.is_clean());
        assert!(!step.after.is_clean());
        assert_eq!(&*log.borrow(), &["enumeration"]);
        let source = outcome.err().unwrap();
        let failure = attempt.finish(Err::<(), _>(source)).err().unwrap();
        assert_eq!(failure.body().unwrap().0.as_ptr(), pointer);
        assert_eq!(
            (failure.report().attempted(), failure.report().unknown()),
            (1, 1)
        );
        assert!(matches!(
            failure.report().first(),
            Some(Issue::Close {
                role: CurrentRole::Enumeration,
                ..
            })
        ));
    }

    #[test]
    fn source_chain_enumeration_open_and_count_keep_distinct_errors() {
        let error = Body("original item error".into());
        let pointer = error.0.as_ptr();
        let collected = collect_source_names([Ok(1u8), Err(error)].into_iter());
        assert_eq!(collected.err().unwrap().0.as_ptr(), pointer);
        assert_eq!(
            count_source_names([Ok(1u8), Err(Body("counted error".into()))].into_iter()),
            2
        );
        let calls = Cell::new(0);
        let mut ended = false;
        let mut items = [
            Ok(Some(b".".to_vec())),
            Ok(Some(b"..".to_vec())),
            Ok(Some(vec![0xff])),
            Err(7u8),
        ]
        .into_iter();
        let mut read = || {
            calls.set(calls.get() + 1);
            items.next().unwrap()
        };
        assert_eq!(
            next_source_name(&mut ended, &mut read),
            Some(Ok(vec![0xff]))
        );
        assert_eq!(next_source_name(&mut ended, &mut read), Some(Err(7)));
        assert_eq!(next_source_name(&mut ended, &mut read), None);
        assert_eq!(calls.get(), 4);
        let mut ended = false;
        assert_eq!(next_source_name(&mut ended, || Ok::<_, u8>(None)), None);
        assert_eq!(
            next_source_name(&mut ended, || panic!("EOF is fused")),
            None::<std::result::Result<Vec<u8>, u8>>
        );
    }

    #[test]
    fn source_chain_child_adoption_precedes_validation_and_parent_retire() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut parent = Some(attempt.adopt(counted(&log, "parent", None), CurrentRole::Ancestor));
        let child = attempt.adopt(counted(&log, "child", None), CurrentRole::Source);
        assert_eq!(attempt.report().live(), 2);
        handoff_current_handle(&mut parent, child, |owner| {
            finish_current_owner(scope, owner, CurrentRole::Ancestor);
        });
        assert_eq!(&*log.borrow(), &["parent"]);
        assert_eq!(attempt.report().live(), 1);
        let policy = Err::<(), _>(Body("validation refused".into()));
        let (policy, step) = retire_current_read(policy, || {
            finish_current_owner(scope, parent.take().unwrap(), CurrentRole::Source)
        });
        assert!(policy.is_err() && step.after.is_clean());
        assert_eq!(&*log.borrow(), &["parent", "child"]);
        assert!(attempt.finish(Err::<(), _>(policy.err().unwrap())).is_err());
    }

    #[test]
    fn source_chain_clone_close_precedes_comparison() {
        for failed_read in [false, true] {
            let attempt = CurrentAttempt::default();
            let invocation = RawInvocation::new(&attempt);
            let scope = CurrentScope {
                invocation: &invocation,
                expected: &attempt,
                captured: &attempt,
            };
            let log = Rc::new(RefCell::new(Vec::new()));
            let clone = attempt.adopt(counted(&log, "clone", None), CurrentRole::Clone);
            let read = if failed_read {
                Err(Body("read".into()))
            } else {
                Ok(Row("bytes".into()))
            };
            let (read, _) = retire_current_read(read, || {
                finish_current_owner(scope, clone, CurrentRole::Clone)
            });
            assert_eq!(&*log.borrow(), &["clone"]);
            if let Ok(row) = &read {
                assert_eq!(&*row.0, "bytes");
                log.borrow_mut().push("compare");
            }
            assert_eq!(log.borrow().len(), if failed_read { 1 } else { 2 });
            assert_eq!(attempt.report().attempted(), 1);
            assert_eq!(attempt.finish(read.map(|_| ())).is_ok(), !failed_read);
        }
    }

    #[test]
    fn source_chain_reverse_cleanup_preserves_prior_events() {
        let attempt = CurrentAttempt::default();
        let invocation = RawInvocation::new(&attempt);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &attempt,
            captured: &attempt,
        };
        let log = Rc::new(RefCell::new(Vec::new()));
        let rows = ["first", "second"]
            .into_iter()
            .map(|name| {
                (
                    attempt.adopt(counted(&log, name, Some(Errno::Io)), CurrentRole::Source),
                    Row(name.into()),
                )
            })
            .collect();
        attempt.gap(CurrentGap::Abandoned);
        let rows = retire_source_rows(rows, |owner| {
            finish_current_owner(scope, owner, CurrentRole::Source);
        });
        assert_eq!(&*log.borrow(), &["second", "first"]);
        assert_eq!((&*rows[0].0, &*rows[1].0), ("first", "second"));
        assert_eq!(
            (attempt.report().live(), attempt.report().unknown()),
            (0, 2)
        );
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }

    #[test]
    fn source_chain_all_foreign_owner_scopes_are_poisoned_even_dirty() {
        let actual = CurrentAttempt::default();
        let expected = CurrentAttempt::default();
        let captured = CurrentAttempt::default();
        let foreign = CurrentAttempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let owner = foreign.adopt(counted(&log, "foreign", None), CurrentRole::Source);
        actual.gap(CurrentGap::Unbound);
        let invocation = RawInvocation::new(&actual);
        let io = CurrentIo::new(CurrentMode::Checked(CurrentScope {
            invocation: &invocation,
            expected: &expected,
            captured: &captured,
        }));
        assert!(check_source_owners(&io, [Some(owner.attempt())].into_iter()).is_err());
        for scope in [&actual, &expected, &captured, &foreign] {
            assert!(!scope.check());
        }
        let _ = io.finish_trace();
        assert_eq!(foreign.report().live(), 1);
        assert!(!owner.finish());
        assert_eq!(foreign.report().attempted(), 1);
        assert_eq!(actual.report().attempted(), 0);
        // Genuine public preflight refuses before path/slot/native effects.
        let actual = CurrentAttempt::default();
        let other = CurrentAttempt::default();
        let invocation = RawInvocation::new(&actual);
        let refusal = ProtectedSourceChain::open_checked_in(
            Path::new("never-opened"),
            "not-a-slot",
            &invocation,
            &other,
            &actual,
        )
        .err()
        .expect("foreign scope refuses first");
        assert_eq!(
            refusal.diagnostics().source_kind(),
            Some(SourceChainSourceKind::Scope)
        );
        assert_eq!(actual.report().attempted(), 0);
        assert_eq!(other.report().attempted(), 0);
        assert!(!actual.check() && !other.check());
    }

    #[test]
    fn source_chain_unwind_gaps_without_terminal_finish() {
        let attempt = CurrentAttempt::default();
        let log = Rc::new(RefCell::new(Vec::new()));
        let outer = attempt.adopt(counted(&log, "outer", None), CurrentRole::Source);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let invocation = RawInvocation::new(&attempt);
            let _io = CurrentIo::new(CurrentMode::Checked(CurrentScope {
                invocation: &invocation,
                expected: &attempt,
                captured: &attempt,
            }));
            panic!("ordinary seam unwind");
        }));
        assert!(result.is_err());
        assert_eq!(attempt.report().live(), 1);
        assert_eq!(attempt.report().attempted(), 0);
        assert_eq!(
            attempt.report().first(),
            Some(Issue::Gap(CurrentGap::Abandoned))
        );
        assert!(!outer.finish());
        assert!(attempt.finish(Ok::<(), Infallible>(())).is_err());
    }

    #[test]
    fn source_chain_debug_has_no_payload_or_body_bounds() {
        let rows = retire_source_rows(
            vec![(Row("resource".into()), Body("secret-body".into()))],
            |_| {},
        );
        assert_eq!(&*rows[0].0, "secret-body");
        let actual = CurrentAttempt::default();
        let other = CurrentAttempt::default();
        let invocation = RawInvocation::new(&actual);
        let failure = ProtectedSourceChain::open_checked_in(
            Path::new("secret-path"),
            "secret-slot",
            &invocation,
            &other,
            &actual,
        )
        .err()
        .unwrap();
        let debug = format!("{failure:?}");
        assert!(debug.contains("CheckedSourceChain"));
        assert!(!debug.contains("secret") && !debug.contains("Report"));
        assert!(failure.diagnostics().policy_source().is_none());
        assert_eq!(failure.diagnostics().event_count(), 0);
    }
}

#[cfg(test)]
mod execution_context_scope_tests {
    use super::{
        CurrentIo, CurrentMode, CurrentScope, CurrentStop, EvidenceError, ExecutionGateFailure,
        ExecutionGateFailureKind, current_failure,
    };
    use crate::{
        checked_close::{Attempt, Failure, Gap},
        raw_custody::RawInvocation,
    };
    use std::convert::Infallible;

    #[test]
    fn dirty_foreign_scopes_all_visited() {
        let actual = Attempt::default();
        let expected = Attempt::default();
        let captured = Attempt::default();
        let held_owner = Attempt::default();
        let invocation = RawInvocation::new(&actual);
        actual.gap(Gap::Abandoned);
        let scope = CurrentScope {
            invocation: &invocation,
            expected: &expected,
            captured: &captured,
        };

        let check = scope.compare(Some(&held_owner));
        assert!(check.expected.is_err());
        assert!(check.captured.is_err());
        assert!(check.owner.as_ref().is_some_and(Result::is_err));
        assert!(!actual.check());
        assert!(!expected.check());
        assert!(!captured.check());
        assert!(!held_owner.check());

        let failure = ExecutionGateFailure::scope(None, None, check);
        assert_eq!(failure.kind(), ExecutionGateFailureKind::Scope);
        assert!(failure.source_report().is_some());
        assert!(failure.current_failure().is_none());
    }

    #[test]
    fn scope_transport_keeps_first_actual_report() {
        let actual = Attempt::default();
        let expected = Attempt::default();
        let captured = Attempt::default();
        let invocation = RawInvocation::new(&actual);
        actual.gap(Gap::Abandoned);
        expected.gap(Gap::ScopeMismatch);
        let first: Failure<Infallible> = actual.failure(None);
        let later: Failure<Infallible> = expected.failure(None);
        let check = CurrentScope {
            invocation: &invocation,
            expected: &expected,
            captured: &captured,
        }
        .compare(None);

        let failure = ExecutionGateFailure::scope(Some(first), Some(later), check);
        assert_eq!(failure.kind(), ExecutionGateFailureKind::Scope);
        let first_report = match failure.source.as_ref() {
            super::ExecutionGateFailureSource::Scope {
                context_invocation: Some(first),
                ..
            } => first.report(),
            _ => panic!("the actual first scope failure is retained"),
        };
        assert!(std::ptr::eq(
            failure.source_report().expect("first report"),
            first_report,
        ));
    }

    #[test]
    fn current_policy_error_keeps_absent_source_report() {
        let io = CurrentIo::new(CurrentMode::Legacy);
        let source = current_failure(&io, Some(CurrentStop::Policy(EvidenceError)), Vec::new());
        let failure = ExecutionGateFailure::current(source);

        assert_eq!(failure.kind(), ExecutionGateFailureKind::Current);
        assert!(failure.current_failure().is_some());
        assert!(failure.source_report().is_none());
    }
}

#[cfg(test)]
mod execution_transport_repair_tests {
    use super::*;
    use crate::checked_close::{Attempt, Close, Errno, Gap, Role};
    use std::{
        cell::{Cell, RefCell},
        convert::Infallible,
        rc::Rc,
    };

    struct Counted(Rc<Cell<usize>>);
    impl Close for Counted {
        fn close(self) -> std::result::Result<(), Errno> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn reentry_returns_each_whole_error_while_outer_owner_stays_live() {
        let attempt = Attempt::default();
        let calls = Rc::new(Cell::new(0));
        let outer = attempt.adopt(Counted(Rc::clone(&calls)), Role::Ancestor);
        let model = RefCell::new(());
        let borrowed = model.borrow_mut();
        let first = borrow_execution_current(&model, &attempt).err().unwrap();
        let later = borrow_execution_current(&model, &attempt).err().unwrap();
        assert_eq!(first.kind(), ExecutionGateFailureKind::Reentrant);
        assert_eq!(later.kind(), ExecutionGateFailureKind::Reentrant);
        assert!(!std::ptr::eq(
            first.source_report().unwrap(),
            later.source_report().unwrap()
        ));
        assert_eq!(first.source_report().unwrap().secondary().len(), 0);
        assert!(!later.source_report().unwrap().secondary().is_empty());
        assert_eq!(attempt.report().live(), 1);
        assert_eq!(calls.get(), 0);
        drop(borrowed);
        assert!(!outer.finish());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn primary_current_policy_none_stays_before_real_later_local_report() {
        let io = CurrentIo::new(CurrentMode::Legacy);
        let first = ExecutionGateFailure::current(current_failure(
            &io,
            Some(CurrentStop::Policy(EvidenceError)),
            Vec::new(),
        ));
        let attempt = Attempt::default();
        let invocation = RawInvocation::new(&attempt);
        attempt.gap(Gap::Reuse);
        let later = invocation.settle_local_in(&attempt, Ok::<(), Infallible>(()));
        assert!(first.current_failure().is_some());
        assert!(first.source_report().is_none());
        assert!(
            later
                .as_ref()
                .err()
                .is_some_and(|f| f.report().first().is_some())
        );
        assert!(first.source_report().is_none());
    }

    #[test]
    fn execution_scope_join_visits_all_dirty_foreign_ledgers() {
        let actual = Attempt::default();
        let context = Attempt::default();
        let expected = Attempt::default();
        let stored_expected = Attempt::default();
        let captured = Attempt::default();
        let held = Attempt::default();
        actual.gap(Gap::Abandoned);
        let invocation = RawInvocation::new(&actual);
        let bootstrap = RawInvocation::new(&context);
        let scope = CurrentScope {
            invocation: &bootstrap,
            expected: &stored_expected,
            captured: &captured,
        };
        let failure =
            compare_execution_scopes(scope, &context, Some(&held), &invocation, &expected).unwrap();
        assert_eq!(failure.kind(), ExecutionGateFailureKind::Scope);
        for attempt in [
            &actual,
            &context,
            &expected,
            &stored_expected,
            &captured,
            &held,
        ] {
            assert!(!attempt.check());
        }
        assert_eq!(actual.report().attempted(), 0);
        assert_eq!(context.report().attempted(), 0);
    }
}

fn execution_snapshot_callback<S, T, E>(
    original: std::result::Result<S, E>,
    callback: impl FnOnce(&S) -> T,
) -> std::result::Result<T, E> {
    original.map(|snapshot| callback(&snapshot))
}
#[cfg(test)]
mod checker_snapshot_tests {
    use super::*;
    use crate::{
        checked_close::{Attempt, Failure, Gap},
        raw_custody::RawInvocation,
    };
    use std::cell::Cell;
    struct Body(Box<str>);
    #[test]
    fn execution_snapshot_callback_uses_same_current_and_keeps_whole_failure() {
        let attempt = Attempt::default();
        let current = std::cell::RefCell::new(Box::<str>::from("same actual model snapshot"));
        let calls = Cell::new(0);
        {
            let held = borrow_execution_current(&current, &attempt).unwrap();
            let pointer = held.as_ptr();
            let snapshot = execution_snapshot_callback(Ok::<_, Failure<Body>>(&*held), |value| {
                calls.set(calls.get() + 1);
                value.as_ptr()
            });
            match snapshot {
                Ok(value) => assert_eq!(value, pointer),
                Err(_) => panic!("ordinary body unexpectedly failed"),
            }
        }
        assert_eq!(calls.get(), 1);
        for some in [true, false] {
            let invocation = RawInvocation::new(&attempt);
            let error = invocation.settle_local_in(
                &attempt,
                if some {
                    Err::<(), _>(Body("whole source".into()))
                } else {
                    attempt.gap(Gap::Unbound);
                    Ok::<(), Body>(())
                },
            );
            let address = error
                .as_ref()
                .err()
                .and_then(Failure::body)
                .map(|body| body.0.as_ptr());
            let result = execution_snapshot_callback(error, |_| {
                calls.set(calls.get() + 1);
            });
            match result {
                Err(error) => {
                    assert_eq!(error.body().is_some(), some);
                    assert_eq!(error.body().map(|body| body.0.as_ptr()), address)
                }
                Ok(()) => panic!("real failure was erased"),
            }
        }
        assert_eq!(calls.get(), 1);
        assert!(
            attempt
                .finish(Ok::<(), std::convert::Infallible>(()))
                .is_err()
        );
    }
}
