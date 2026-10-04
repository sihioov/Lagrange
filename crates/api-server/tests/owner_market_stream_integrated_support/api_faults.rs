//! Two explicit fault actions for the owned synthetic integration fixture.
//! Default positive fixtures never read these control files or execute this SQL.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Scenario {
    ListenCache,
    ApiRestart,
    SlowConsumer,
    WindowCycle,
    RightsRevocation,
    GenerationChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    ListenerLoss,
    CacheRecreation,
    ApiRestart,
    SlowBaseline,
    SlowObserve,
    WindowWithdraw,
    WindowRestore,
    RevokeRights,
    VerifyRevoked,
    SupersedeGeneration,
    VerifySuperseded,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: u32,
    sequence: u8,
    kind: Kind,
}

impl OwnedFixture {
    pub(super) async fn poll_api_fault(&mut self) -> FixtureResult {
        let expected = match (self.settings.fault_scenario, self.api_fault_step) {
            (Some(Scenario::ListenCache), 0) => Kind::ListenerLoss,
            (Some(Scenario::ListenCache), 1) => Kind::CacheRecreation,
            (Some(Scenario::ApiRestart), 0) => Kind::ApiRestart,
            (Some(Scenario::SlowConsumer), 0) => Kind::SlowBaseline,
            (Some(Scenario::SlowConsumer), 1) => Kind::SlowObserve,
            (Some(Scenario::WindowCycle), 0) => Kind::WindowWithdraw,
            (Some(Scenario::WindowCycle), 1) => Kind::WindowRestore,
            (Some(Scenario::RightsRevocation), 0) => Kind::RevokeRights,
            (Some(Scenario::RightsRevocation), 1) => Kind::VerifyRevoked,
            (Some(Scenario::GenerationChange), 0) => Kind::SupersedeGeneration,
            (Some(Scenario::GenerationChange), 1) => Kind::VerifySuperseded,
            _ => return Ok(()),
        };
        if self.api_fault_step >= 2 {
            return Ok(());
        }
        let sequence = self.api_fault_step + 1;
        let path = self
            .settings
            .directory
            .join(format!("fault-{sequence:02}.json"));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(FixtureFailure::ControlInvalid),
            Ok(_) => {}
        }
        let bytes = read_private_file(&path, 256)?;
        let request: Request =
            serde_json::from_slice(&bytes).map_err(|_| FixtureFailure::ControlInvalid)?;
        if request.schema_version != 1 || request.sequence != sequence || request.kind != expected {
            return Err(FixtureFailure::ControlInvalid);
        }
        // Consume before executing. An error terminates the fixture, never retries the action.
        self.api_fault_step = sequence;
        let detail = match request.kind {
            Kind::ListenerLoss => self.break_owned_listener().await?,
            Kind::CacheRecreation => self.recreate_owned_cache().await?,
            Kind::ApiRestart => self.restart_owned_api().await?,
            Kind::SlowBaseline => self.capture_slow_baseline().await?,
            Kind::SlowObserve => self.observe_slow_consumer().await?,
            Kind::WindowWithdraw => self.set_owned_window_visible(false)?,
            Kind::WindowRestore => self.set_owned_window_visible(true)?,
            Kind::RevokeRights => self.revoke_owned_rights().await?,
            Kind::VerifyRevoked => self.verify_owned_rights_revoked().await?,
            Kind::SupersedeGeneration => self.supersede_owned_generation().await?,
            Kind::VerifySuperseded => self.verify_owned_generation_superseded().await?,
        };
        atomic_json_once(
            &self
                .settings
                .directory
                .join(format!("fault-{sequence:02}-result.json")),
            &json!({"schema_version": 1, "sequence": sequence, "kind": request.kind,
                "status": "PASS", "detail": detail}),
        )
    }

    async fn old_generation_quote_version(&self) -> FixtureResult<Option<i64>> {
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let fixture = self.fixture.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let identity = fixture
            .identities
            .first()
            .ok_or(FixtureFailure::SetupFailed)?;
        let generation =
            i64::try_from(identity.generation).map_err(|_| FixtureFailure::ControlInvalid)?;
        timeout_at(
            Instant::now() + OBSERVATION_BOUND,
            sqlx::query_scalar(
                "SELECT quote_version FROM public.owner_market_stream_cache
                 WHERE owner_user_id = $1 AND credential_slot_id = $2
                   AND membership_id = $3 AND generation = $4",
            )
            .bind(fixture.owner_user_id)
            .bind(fixture.credential_slot_id)
            .bind(identity.membership_id)
            .bind(generation)
            .fetch_optional(&database.migration_owner),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)?
        .map_err(|_| FixtureFailure::ObservationFailed)
    }

    async fn supersede_owned_generation(&mut self) -> FixtureResult<Value> {
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let fixture = self.fixture.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let identity = fixture
            .identities
            .first()
            .ok_or(FixtureFailure::SetupFailed)?;
        let old_generation = identity.generation;
        timeout_at(
            Instant::now() + OBSERVATION_BOUND,
            boundary::supersede_generation(database, identity),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)?
        .map_err(|_| FixtureFailure::ObservationFailed)?;
        self.generation_quote_version = self.old_generation_quote_version().await?;
        Ok(
            json!({"memberships_advanced": 1, "previous_generation": old_generation,
            "replacement_generation": old_generation + 1}),
        )
    }

    async fn verify_owned_generation_superseded(&self) -> FixtureResult<Value> {
        let current = self.old_generation_quote_version().await?;
        if current.is_some() && self.generation_quote_version != current {
            return Err(FixtureFailure::ObservationFailed);
        }
        Ok(json!({"old_generation_row_absent_or_version_unchanged": true,
            "old_generation_row_absent": current.is_none(), "sql_mutations": 0}))
    }

    async fn revoke_owned_rights(&mut self) -> FixtureResult<Value> {
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let fixture = self.fixture.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let changed = timeout_at(
            Instant::now() + OBSERVATION_BOUND,
            sqlx::query(
                "UPDATE public.data_entitlements SET status = 'REVOKED'
                 WHERE status = 'ACTIVE' AND id = (
                     SELECT entitlement_id FROM public.owner_market_stream_grants
                     WHERE id = $1 AND owner_user_id = $2 AND credential_slot_id = $3
                 )",
            )
            .bind(fixture.grant_id)
            .bind(fixture.owner_user_id)
            .bind(fixture.credential_slot_id)
            .execute(&database.migration_owner),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)?
        .map_err(|_| FixtureFailure::ObservationFailed)?;
        if changed.rows_affected() != 1 {
            return Err(FixtureFailure::ControlInvalid);
        }
        let after = timeout_at(
            Instant::now() + OBSERVATION_BOUND,
            query_observation(database, fixture.owner_user_id, fixture.credential_slot_id),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)??;
        self.rights_quote_version = Some(after.quote_version_sum);
        let task = self
            .daemon_task
            .as_mut()
            .ok_or(FixtureFailure::DaemonFailed)?;
        let result = timeout_at(Instant::now() + Duration::from_secs(10), task)
            .await
            .map_err(|_| FixtureFailure::ObservationFailed)?;
        self.daemon_task.take();
        let code = match result {
            Ok(Err(MarketStreamRuntimeError::GrantUnavailable)) => "GRANT_UNAVAILABLE",
            Ok(Err(MarketStreamRuntimeError::Terminal)) => "TERMINAL",
            _ => return Err(FixtureFailure::DaemonFailed),
        };
        self.daemon_exit = "RIGHTS_DENIAL_JOINED";
        let transport = self.metrics.snapshot()?;
        if transport.active_websockets != 0 || !transport.client_close_seen {
            return Err(FixtureFailure::WebsocketFailed);
        }
        Ok(json!({"entitlements_changed": 1, "daemon_joined": true,
            "runtime_outcome": code, "upstream_closed": true, "forced_abort": false}))
    }

    async fn verify_owned_rights_revoked(&self) -> FixtureResult<Value> {
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let fixture = self.fixture.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let after = timeout_at(
            Instant::now() + OBSERVATION_BOUND,
            query_observation(database, fixture.owner_user_id, fixture.credential_slot_id),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)??;
        if self.rights_quote_version != Some(after.quote_version_sum)
            || self.daemon_task.is_some()
            || self.daemon_exit != "RIGHTS_DENIAL_JOINED"
            || self.metrics.snapshot()?.active_websockets != 0
        {
            return Err(FixtureFailure::ObservationFailed);
        }
        Ok(json!({"post_revoke_quote_versions_unchanged": true,
            "runtime_joined": true, "active_websockets": 0,
            "retained_desired_count": after.desired_count, "sql_mutations": 0}))
    }

    fn set_owned_window_visible(&self, visible: bool) -> FixtureResult<Value> {
        let sender = self
            .window_watch
            .as_ref()
            .ok_or(FixtureFailure::SetupFailed)?;
        if sender.is_closed() || sender.borrow().is_some() == visible {
            return Err(FixtureFailure::ControlInvalid);
        }
        let replacement = if visible {
            Some(
                self.window
                    .as_ref()
                    .ok_or(FixtureFailure::WindowInvalid)?
                    .as_ref()
                    .clone(),
            )
        } else {
            None
        };
        let previous = sender.send_replace(replacement);
        if previous.is_some() == visible {
            return Err(FixtureFailure::ControlInvalid);
        }
        Ok(json!({"window_present": visible, "original_owned_window": true, "sql_mutations": 0}))
    }

    async fn owned_listener_pids(&self, deadline: Instant) -> FixtureResult<Vec<i32>> {
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        timeout_at(
            deadline.min(Instant::now() + OBSERVATION_BOUND),
            sqlx::query_scalar(
                "SELECT pid FROM pg_catalog.pg_stat_activity
                 WHERE datname = current_database() AND usename = current_user
                   AND backend_type = 'client backend' AND query = $1 AND state = 'idle'
                 ORDER BY pid",
            )
            .bind("LISTEN \"owner_market_stream_changed\"")
            .fetch_all(&database.app),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)?
        .map_err(|_| FixtureFailure::ObservationFailed)
    }

    async fn capture_slow_baseline(&mut self) -> FixtureResult<Value> {
        if self.slow_listener_baseline.is_some() {
            return Err(FixtureFailure::ControlInvalid);
        }
        let pids = self
            .owned_listener_pids(Instant::now() + OBSERVATION_BOUND)
            .await?;
        if pids.len() != 1 {
            return Err(FixtureFailure::ObservationFailed);
        }
        self.slow_listener_baseline = Some(pids[0]);
        Ok(json!({"original_listeners": 1}))
    }

    async fn observe_slow_consumer(&self) -> FixtureResult<Value> {
        let baseline = self
            .slow_listener_baseline
            .ok_or(FixtureFailure::ControlInvalid)?;
        let started = Instant::now();
        let deadline = started + Duration::from_secs(25);
        let initial = self.owned_listener_pids(deadline).await?;
        if initial.len() != 2 || !initial.contains(&baseline) {
            return Err(FixtureFailure::ObservationFailed);
        }
        let target = *initial
            .iter()
            .find(|pid| **pid != baseline)
            .ok_or(FixtureFailure::ObservationFailed)?;
        atomic_json_once(
            &self.settings.directory.join("slow-start.json"),
            &json!({"baseline_pid": baseline, "target_pid": target}),
        )?;
        let mut observations = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.settings.directory.join("slow-observations.jsonl"))
            .map_err(|_| FixtureFailure::ControlInvalid)?;
        let mut records = 0;
        loop {
            let pids = self.owned_listener_pids(deadline).await?;
            records += 1;
            if records > 512 {
                return Err(FixtureFailure::ObservationLimit);
            }
            writeln!(
                observations,
                "{}",
                json!({"elapsed_ms": started.elapsed().as_millis(), "listener_pids": pids})
            )
            .map_err(|_| FixtureFailure::ObservationFailed)?;
            if !pids.contains(&baseline) || pids.iter().any(|p| *p != baseline && *p != target) {
                return Err(FixtureFailure::ObservationFailed);
            }
            if !pids.contains(&target) {
                let elapsed_ms = started.elapsed().as_millis();
                if pids != vec![baseline] || elapsed_ms < 4_500 {
                    return Err(FixtureFailure::ObservationFailed);
                }
                return Ok(json!({"initial_listeners": 2, "remaining_listeners": 1,
                    "original_listener_preserved": true, "slow_listener_released": true,
                    "unread_observation_ms": elapsed_ms, "sql_mutations": 0}));
            }
            if Instant::now() >= deadline {
                return Err(FixtureFailure::ObservationFailed);
            }
            sleep_until((Instant::now() + Duration::from_millis(50)).min(deadline)).await;
        }
    }

    async fn restart_owned_api(&mut self) -> FixtureResult<Value> {
        let socket = self
            .api_socket
            .as_ref()
            .ok_or(FixtureFailure::SetupFailed)?
            .clone();
        if socket != self.settings.directory.join("api.sock") {
            return Err(FixtureFailure::ControlInvalid);
        }
        let old = fs::symlink_metadata(&socket).map_err(|_| FixtureFailure::ControlInvalid)?;
        if !old.file_type().is_socket()
            || old.uid() != effective_uid()
            || old.mode() & 0o7777 != 0o600
        {
            return Err(FixtureFailure::ControlInvalid);
        }
        // The real offline browser has already closed its response. Never detach
        // the old server or replace it before all of its HTTP connections joined.
        self.api_shutdown
            .as_ref()
            .ok_or(FixtureFailure::SetupFailed)?
            .send_replace(true);
        let task = self.api_task.take().ok_or(FixtureFailure::SetupFailed)?;
        join_fixture_task(
            task,
            Instant::now() + Duration::from_secs(5),
            FixtureFailure::ServerFailed,
            &mut self.forced_abort,
        )
        .await?;
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let deadline = Instant::now() + OBSERVATION_BOUND;
        loop {
            let listeners: i64 = timeout_at(deadline, sqlx::query_scalar(
                "SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity
                 WHERE datname = current_database() AND query = $1 AND backend_type = 'client backend'",
            ).bind("LISTEN \"owner_market_stream_changed\"").fetch_one(&database.migration_owner))
                .await.map_err(|_| FixtureFailure::ObservationFailed)?
                .map_err(|_| FixtureFailure::ObservationFailed)?;
            if listeners == 0 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(FixtureFailure::ObservationFailed);
            }
            sleep_until((Instant::now() + Duration::from_millis(5)).min(deadline)).await;
        }
        let current = fs::symlink_metadata(&socket).map_err(|_| FixtureFailure::ControlInvalid)?;
        if !current.file_type().is_socket()
            || current.dev() != old.dev()
            || current.ino() != old.ino()
        {
            return Err(FixtureFailure::ControlInvalid);
        }
        fs::remove_file(&socket).map_err(|_| FixtureFailure::CleanupFailed)?;
        // Construct fresh in-memory API state; role pools and synthetic durable
        // session/lease rows retain the same identity, as with a normal restart.
        let config = self
            .restart_config
            .as_ref()
            .ok_or(FixtureFailure::SetupFailed)?
            .clone();
        let audit = self
            .audit_pool
            .as_ref()
            .ok_or(FixtureFailure::SetupFailed)?
            .clone();
        let state = timeout_at(
            deadline,
            ApiState::from_pools(config, database.app.clone(), database.admin.clone(), audit),
        )
        .await
        .map_err(|_| FixtureFailure::ApiSetupFailed)?
        .map_err(|_| FixtureFailure::ApiSetupFailed)?;
        timeout_at(deadline, state.check_readiness())
            .await
            .map_err(|_| FixtureFailure::ApiSetupFailed)?
            .map_err(|_| FixtureFailure::ApiSetupFailed)?;
        let listener =
            UnixListener::bind(&socket).map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|_| FixtureFailure::ListenerSetupFailed)?;
        let router = api_router(state);
        let (stop, stop_rx) = watch::channel(false);
        self.api_shutdown = Some(stop);
        let active = Arc::clone(&self.active_tasks);
        self.api_task = Some(tokio::spawn(async move {
            let _active = ActiveTask::enter(active);
            axum::serve(listener, router)
                .with_graceful_shutdown(wait_for_true(stop_rx))
                .await
                .map_err(|_| FixtureFailure::ServerFailed)
        }));
        Ok(json!({"old_server_joined": true, "old_listener_gone": true,
            "fresh_api_state": true, "same_owned_endpoint": true, "new_server_started": true}))
    }

    async fn break_owned_listener(&self) -> FixtureResult<Value> {
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        const LISTEN: &str = "LISTEN \"owner_market_stream_changed\"";
        let deadline = Instant::now() + OBSERVATION_BOUND;
        let targets: Vec<i32> = timeout_at(
            deadline,
            sqlx::query_scalar(
                "SELECT pid FROM pg_catalog.pg_stat_activity
             WHERE datname = current_database() AND usename = current_user
               AND backend_type = 'client backend' AND query = $1 AND state = 'idle'",
            )
            .bind(LISTEN)
            .fetch_all(&database.app),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)?
        .map_err(|_| FixtureFailure::ObservationFailed)?;
        if targets.len() != 1 {
            return Err(FixtureFailure::ObservationFailed);
        }
        let pid = targets[0];
        let terminated: Option<bool> = timeout_at(
            deadline,
            sqlx::query_scalar(
                "SELECT pg_catalog.pg_terminate_backend(pid)
             FROM pg_catalog.pg_stat_activity
             WHERE pid = $1 AND datname = current_database() AND usename = current_user
               AND backend_type = 'client backend' AND query = $2 AND state = 'idle'",
            )
            .bind(pid)
            .bind(LISTEN)
            .fetch_optional(&database.app),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)?
        .map_err(|_| FixtureFailure::ObservationFailed)?;
        if terminated != Some(true) {
            return Err(FixtureFailure::ObservationFailed);
        }
        loop {
            let remains: bool = timeout_at(
                deadline,
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity
                 WHERE pid = $1 AND datname = current_database())",
                )
                .bind(pid)
                .fetch_one(&database.migration_owner),
            )
            .await
            .map_err(|_| FixtureFailure::ObservationFailed)?
            .map_err(|_| FixtureFailure::ObservationFailed)?;
            if !remains {
                break;
            }
            sleep_until((Instant::now() + Duration::from_millis(5)).min(deadline)).await;
        }
        Ok(json!({"owned_listener_count": 1, "terminated": true, "pid_absent": true}))
    }

    async fn recreate_owned_cache(&self) -> FixtureResult<Value> {
        let database = self.database.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let fixture = self.fixture.as_ref().ok_or(FixtureFailure::SetupFailed)?;
        let identity = fixture
            .identities
            .first()
            .ok_or(FixtureFailure::SetupFailed)?;
        let deadline = Instant::now() + OBSERVATION_BOUND;
        let removed: Vec<(Uuid, i64)> = timeout_at(
            deadline,
            sqlx::query_as(
                "DELETE FROM public.owner_market_stream_cache
             WHERE owner_user_id = $1 AND credential_slot_id = $2 AND membership_id = $3
             RETURNING row_generation, quote_version",
            )
            .bind(fixture.owner_user_id)
            .bind(fixture.credential_slot_id)
            .bind(identity.membership_id)
            .fetch_all(&database.migration_owner),
        )
        .await
        .map_err(|_| FixtureFailure::ObservationFailed)?
        .map_err(|_| FixtureFailure::ObservationFailed)?;
        if removed.len() != 1 || removed[0].1 <= 4 {
            return Err(FixtureFailure::ObservationFailed);
        }
        // Only the actual producer recreates the row from a fresh authenticated receipt.
        loop {
            let current: Option<(Uuid, i64)> = timeout_at(
                deadline,
                sqlx::query_as(
                    "SELECT row_generation, quote_version FROM public.owner_market_stream_cache
                 WHERE owner_user_id = $1 AND credential_slot_id = $2 AND membership_id = $3",
                )
                .bind(fixture.owner_user_id)
                .bind(fixture.credential_slot_id)
                .bind(identity.membership_id)
                .fetch_optional(&database.migration_owner),
            )
            .await
            .map_err(|_| FixtureFailure::ObservationFailed)?
            .map_err(|_| FixtureFailure::ObservationFailed)?;
            if let Some((generation, version)) = current {
                if generation == removed[0].0 || version <= 0 || version >= removed[0].1 {
                    return Err(FixtureFailure::ObservationFailed);
                }
                return Ok(json!({"removed_rows": 1, "row_generation_changed": true,
                    "quote_version_restarted": true, "instrument": identity.instrument_id,
                    "prior_version": removed[0].1, "recreated_version": version}));
            }
            sleep_until((Instant::now() + Duration::from_millis(5)).min(deadline)).await;
        }
    }
}
