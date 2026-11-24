-- Reconcile only canonical revocations of a protected, initial OFF account.
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT CASE WHEN current_user='migration_owner' AND session_user=current_user
  AND current_setting('role')='none' AND current_setting('transaction_read_only')='on'
  AND EXISTS (SELECT 1 FROM public._sqlx_migrations m WHERE m.version=70 AND m.success
    AND m.checksum=decode('dacd033fc2e4006420621aad20fea28c4262de953924a056980c0e584c753d3b1a9b949d163f4c73196507276b0eae4a','hex'))
  AND EXISTS (
    SELECT 1 FROM public.owner_beta_paper_control c
    WHERE c.id='etf11' AND c.account_id=__ACCOUNT_SQL__::uuid
      AND c.control_epoch=__CONTROL_EPOCH__ AND c.state='OFF'
      AND c.activation_event_id IS NULL AND c.process_binding_sha256 IS NULL
      AND c.cohort_sha256 IS NULL
      AND EXISTS (SELECT 1 FROM public.owner_beta_paper_events o
        WHERE o.owner_user_id=c.owner_user_id AND o.account_id=c.account_id AND o.control_epoch=0
          AND o.kind='CONFIGURED' AND o.payload->>'strategy_config_id'=c.strategy_config_id::text
          AND o.payload->'contract'=c.contract_json)
      AND NOT EXISTS (SELECT 1 FROM public.owner_beta_paper_events e
        WHERE e.account_id=c.account_id AND e.kind IN ('ENABLED','RESTORED_OFF','RECOVERY_AUTHORIZED','BLOCKED'))
      AND NOT EXISTS (SELECT 1 FROM public.owner_beta_paper_events e
        WHERE e.account_id=c.account_id AND e.control_epoch>__LOCAL_FLOOR__ AND e.control_epoch<=__CONTROL_EPOCH__
          AND e.kind NOT IN ('CONSENT_GRANTED','CONSENT_REVOKED'))
      AND (SELECT count(*)=__REVOCATIONS__ AND count(DISTINCT e.control_epoch)=__REVOCATIONS__
        FROM public.owner_beta_paper_events e
        WHERE e.owner_user_id=c.owner_user_id AND e.account_id=c.account_id
          AND e.actor_user_id=c.owner_user_id AND e.kind='CONSENT_REVOKED'
          AND e.control_epoch>__LOCAL_FLOOR__ AND e.control_epoch<=__CONTROL_EPOCH__
          AND e.request_key LIKE 'api-paper-consent:%'
          AND e.payload_sha256=public.daily_0060_digest(e.payload::text)
          AND e.payload=jsonb_build_object('schema','owner-beta-paper-consent-v1',
            'opt_in',false,'expected_epoch',e.control_epoch-1,'resulting_epoch',e.control_epoch,
            'consent_version',c.contract_id,'contract_sha256',c.contract_sha256))
  ) THEN 'OFF_CONSENT_PENDING' ELSE 'NO' END;
ROLLBACK;
