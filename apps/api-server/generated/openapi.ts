export interface paths {
    "/api/v1/auth/session": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/auth/session */
        get: operations["get__api_v1_auth_session"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/logout": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/auth/logout */
        post: operations["post__api_v1_auth_logout"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/csrf": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/auth/csrf */
        get: operations["get__api_v1_auth_csrf"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/auth/step-up-check": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/auth/step-up-check */
        get: operations["get__api_v1_auth_step_up_check"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/strategies": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/strategies */
        get: operations["get__api_v1_strategies"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/strategies/{strategy_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/strategies/{strategy_id} */
        get: operations["get__api_v1_strategies__strategy_id_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/strategies/{strategy_id}/configs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/strategies/{strategy_id}/configs */
        post: operations["post__api_v1_strategies__strategy_id__configs"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/strategy-configs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/strategy-configs */
        get: operations["get__api_v1_strategy_configs"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/strategy-configs/{config_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/strategy-configs/{config_id} */
        get: operations["get__api_v1_strategy_configs__config_id_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/recommendations/runs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/recommendations/runs */
        get: operations["get__api_v1_recommendations_runs"];
        put?: never;
        /** POST /api/v1/recommendations/runs */
        post: operations["post__api_v1_recommendations_runs"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/recommendations/runs/{run_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/recommendations/runs/{run_id} */
        get: operations["get__api_v1_recommendations_runs__run_id_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/recommendations/latest": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/recommendations/latest */
        get: operations["get__api_v1_recommendations_latest"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/backtests */
        get: operations["get__api_v1_backtests"];
        put?: never;
        /** POST /api/v1/backtests */
        post: operations["post__api_v1_backtests"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests/{run_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/backtests/{run_id} */
        get: operations["get__api_v1_backtests__run_id_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests/{run_id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/backtests/{run_id}/cancel */
        post: operations["post__api_v1_backtests__run_id__cancel"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests/{run_id}/metrics": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/backtests/{run_id}/metrics */
        get: operations["get__api_v1_backtests__run_id__metrics"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests/{run_id}/equity": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/backtests/{run_id}/equity */
        get: operations["get__api_v1_backtests__run_id__equity"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests/{run_id}/trades": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/backtests/{run_id}/trades */
        get: operations["get__api_v1_backtests__run_id__trades"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests/{run_id}/robustness": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/backtests/{run_id}/robustness */
        post: operations["post__api_v1_backtests__run_id__robustness"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/backtests/compare": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/backtests/compare */
        post: operations["post__api_v1_backtests_compare"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts */
        get: operations["get__api_v1_paper_accounts"];
        put?: never;
        /** POST /api/v1/paper/accounts */
        post: operations["post__api_v1_paper_accounts"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id} */
        get: operations["get__api_v1_paper_accounts__account_id_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/bind-strategy": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/paper/accounts/{account_id}/bind-strategy */
        post: operations["post__api_v1_paper_accounts__account_id__bind_strategy"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/orders": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id}/orders */
        get: operations["get__api_v1_paper_accounts__account_id__orders"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/positions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id}/positions */
        get: operations["get__api_v1_paper_accounts__account_id__positions"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/equity": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id}/equity */
        get: operations["get__api_v1_paper_accounts__account_id__equity"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/performance": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id}/performance */
        get: operations["get__api_v1_paper_accounts__account_id__performance"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/lineage": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id}/lineage */
        get: operations["get__api_v1_paper_accounts__account_id__lineage"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/parity": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id}/parity */
        get: operations["get__api_v1_paper_accounts__account_id__parity"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/datasets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/datasets */
        get: operations["get__api_v1_admin_datasets"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/datasets/{dataset_id}/approve": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/datasets/{dataset_id}/approve */
        post: operations["post__api_v1_admin_datasets__dataset_id__approve"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/datasets/{dataset_id}/block": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/datasets/{dataset_id}/block */
        post: operations["post__api_v1_admin_datasets__dataset_id__block"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/jobs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/jobs */
        get: operations["get__api_v1_admin_jobs"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/jobs/{job_id}/retry": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/jobs/{job_id}/retry */
        post: operations["post__api_v1_admin_jobs__job_id__retry"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/workers": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/workers */
        get: operations["get__api_v1_admin_workers"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/users": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/users */
        get: operations["get__api_v1_admin_users"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/audit-logs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/audit-logs */
        get: operations["get__api_v1_admin_audit_logs"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/notifications/deliveries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/notifications/deliveries */
        get: operations["get__api_v1_admin_notifications_deliveries"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/notifications */
        get: operations["get__api_v1_notifications"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications/subscriptions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/notifications/subscriptions */
        get: operations["get__api_v1_notifications_subscriptions"];
        /** PUT /api/v1/notifications/subscriptions */
        put: operations["put__api_v1_notifications_subscriptions"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/notifications/test": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/notifications/test */
        post: operations["post__api_v1_notifications_test"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/live/connections": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/live/connections */
        get: operations["get__api_v1_admin_live_connections"];
        put?: never;
        /** POST /api/v1/admin/live/connections */
        post: operations["post__api_v1_admin_live_connections"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/live/orders": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/live/orders */
        post: operations["post__api_v1_admin_live_orders"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/live/connections/{connection_id}/start": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/live/connections/{connection_id}/start */
        post: operations["post__api_v1_admin_live_connections__connection_id__start"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/live/nodes/{node_id}/stop": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/live/nodes/{node_id}/stop */
        post: operations["post__api_v1_admin_live_nodes__node_id__stop"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/live/kill-switch/enable": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/live/kill-switch/enable */
        post: operations["post__api_v1_admin_live_kill_switch_enable"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/live/kill-switch/disable": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/admin/live/kill-switch/disable */
        post: operations["post__api_v1_admin_live_kill_switch_disable"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/admin/live/reconciliation": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/admin/live/reconciliation */
        get: operations["get__api_v1_admin_live_reconciliation"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/licensing-status": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/licensing-status */
        get: operations["get__api_v1_licensing_status"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/metrics": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/metrics */
        get: operations["get__api_v1_metrics"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/artifacts/{artifact_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/artifacts/{artifact_id} */
        get: operations["get__api_v1_artifacts__artifact_id_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/artifacts/{artifact_id}/download": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/artifacts/{artifact_id}/download */
        get: operations["get__api_v1_artifacts__artifact_id__download"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        Error: {
            code: components["schemas"]["ErrorCode"];
            message: string;
            /** @description X-Request-Id echoed into the envelope */
            request_id: string;
            details?: {
                [key: string]: unknown;
            };
        };
        ErrorEnvelope: {
            error: components["schemas"]["Error"];
        };
        /** @enum {string} */
        ErrorCode: "SESSION_UNKNOWN" | "SESSION_EXPIRED" | "FORBIDDEN" | "DATA_ENTITLEMENT_REQUIRED" | "OWNER_ONLY_DEVELOPMENT_PATH" | "CSRF_DENIED" | "STEP_UP_NOT_OWNER" | "STEP_UP_MFA_REQUIRED" | "STEP_UP_AUTH_TIME_ABSENT" | "STEP_UP_AUTH_TIME_STALE" | "RESOURCE_NOT_FOUND" | "INVALID_PARAMETER" | "INVALID_DATE" | "INVALID_DECIMAL" | "INVALID_CURSOR" | "IDEMPOTENCY_KEY_REQUIRED" | "IDEMPOTENCY_KEY_MISMATCH" | "DUPLICATE_RESOURCE" | "PAYLOAD_TOO_LARGE" | "DATASET_BLOCKED" | "DATA_STALE" | "INVALID_STRATEGY_PARAMETER" | "UNSUPPORTED_MARKET_CURRENCY" | "BACKTEST_CAPACITY_EXCEEDED" | "RECOMMENDATION_CAPACITY_EXCEEDED" | "RESULT_INTEGRITY_FAILED" | "LIVE_RECONCILIATION_REQUIRED" | "LIVE_KILL_SWITCH_ENGAGED" | "LIVE_CONNECTION_NOT_CONFIGURED" | "RISK_LIMIT_EXCEEDED" | "ORDER_STATE_UNKNOWN" | "NOT_IMPLEMENTED" | "INTERNAL";
        Page: {
            items: Record<string, never>[];
            /** @description opaque signed cursor; null when the last page */
            next_cursor: string | null;
            has_more: boolean;
        };
        Strategy: {
            id: string;
            display_name: string;
            description?: string;
            risk_description?: string;
            /** @enum {string} */
            state: "Draft" | "Validated" | "Paper" | "LiveCandidate" | "Retired";
            latest_version?: string | null;
        };
        StrategyConfig: {
            /** Format: uuid */
            id: string;
            strategy_id: string;
            strategy_version: string;
            /** @description schema-bound parameters only; code is never accepted */
            config: {
                [key: string]: unknown;
            };
            is_active: boolean;
            /** Format: date-time */
            created_at?: string;
            /** Format: date-time */
            updated_at?: string;
        };
        NewStrategyConfigBody: {
            strategy_version: string;
            config: {
                [key: string]: unknown;
            };
            /** @default true */
            is_active: boolean;
        };
        RecommendationItem: {
            /** @example 069500.KRX */
            instrument_id: string;
            rank?: number | null;
            /** @description decimal string (scale 6) */
            target_weight?: string | null;
            excluded: boolean;
            exclusion_reason?: string | null;
            reason_codes?: string[];
            factors?: {
                [key: string]: unknown;
            };
        };
        RecommendationRun: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            strategy_config_id: string | null;
            /** @example 2026-01-31 */
            as_of: string;
            /** @enum {string} */
            status: "PENDING" | "SUCCEEDED" | "FAILED" | "BLOCKED";
            summary: {
                [key: string]: unknown;
            };
            /** Format: date-time */
            created_at: string;
            /** @enum {string} */
            trigger_kind: "MANUAL" | "SCHEDULED";
            provenance: components["schemas"]["RecommendationProvenance"];
            /** Format: uuid */
            job_id?: string | null;
            items?: components["schemas"]["RecommendationItem"][];
        };
        RecommendationProvenance: {
            /** Format: uuid */
            dataset_version_id?: string;
            dataset_manifest_sha256?: string;
        };
        RecommendationRunPage: {
            items: components["schemas"]["RecommendationRun"][];
            next_cursor: string | null;
            has_more: boolean;
        };
        RecommendationLatest: {
            run: components["schemas"]["RecommendationRun"] | null;
            latest_run: components["schemas"]["RecommendationRun"];
        };
        RecommendationRunBody: {
            /** Format: uuid */
            strategy_config_id: string;
            /** @example 2026-01-31 */
            as_of: string;
        };
        BacktestRun: {
            /** Format: uuid */
            id: string;
            strategy_id: string;
            strategy_version: string;
            dataset_version?: string;
            engine?: string;
            engine_version?: string;
            /** @enum {string} */
            status: "PENDING" | "RUNNING" | "SUCCEEDED" | "FAILED" | "CANCELED";
            /** Format: uuid */
            job_id?: string | null;
            config_sha256?: string;
            benchmark?: string | null;
            start_date?: string | null;
            end_date?: string | null;
            /** Format: date-time */
            started_at?: string | null;
            /** Format: date-time */
            finished_at?: string | null;
            /** Format: date-time */
            created_at?: string;
            summary?: {
                [key: string]: unknown;
            };
        };
        BacktestBody: {
            /** Format: uuid */
            strategy_config_id: string;
            /** Format: uuid */
            dataset_version_id: string;
            /** @example 2026-01-31 */
            start_date: string;
            /** @example 2026-01-31 */
            end_date: string;
            initial_cash: {
                /**
                 * @description KRW first; unsupported currencies are 422 UNSUPPORTED_MARKET_CURRENCY
                 * @enum {string}
                 */
                currency: "KRW";
                /** @description fixed-point decimal string */
                amount: string;
            };
            /**
             * @description member of the fixed Korean ETF v1 universe
             * @example 069500.KRX
             */
            benchmark: string;
            /**
             * @description Same identities as an account's profile; CUSTOM is not yet selectable and is rejected rather than substituted
             * @enum {string}
             */
            cost_profile_id: "KRX_ETF_DEFAULT" | "CUSTOM";
            /** @example daily-close-next-open@1 */
            execution_profile: string;
            /** @default false */
            robustness: boolean;
        };
        Metric: {
            metric_key: string;
            /** @description decimal string */
            metric_value: string;
        };
        Artifact: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            run_id: string;
            /** @enum {string} */
            artifact_type: "EQUITY_CURVE" | "DRAWDOWN_CURVE" | "MONTHLY_RETURNS" | "ORDERS" | "FILLS" | "POSITIONS" | "CASH_LEDGER" | "FEES" | "BENCHMARK";
            row_count: number;
            sha256: string;
            size_bytes: number;
            summary?: {
                [key: string]: unknown;
            };
            /** @description versioned API path; never a filesystem URL */
            download_path: string;
        };
        Equity: {
            /** Format: uuid */
            run_id: string;
            artifact: components["schemas"]["Artifact"];
            summary: {
                [key: string]: unknown;
            };
        };
        Trade: {
            /** Format: uuid */
            run_id: string;
            artifact_type: string;
            artifact: components["schemas"]["Artifact"];
        };
        Compare: {
            run_ids: string[];
            runs: {
                /** Format: uuid */
                run_id?: string;
                strategy_id?: string;
                status?: string;
                summary?: Record<string, never>;
            }[];
            deltas: {
                [key: string]: string;
            };
        };
        CompareBody: {
            run_ids: string[];
        };
        Cancel: {
            /** Format: uuid */
            run_id: string;
            /** Format: uuid */
            job_id?: string | null;
            /** @enum {string} */
            status: "CANCEL_REQUESTED";
        };
        /** @description One entry per requested derived-run child; each entry changes exactly one axis (design 9.5). Omitting axes runs the standard adverse/extreme cost-stress pair. A canceled parent run cascades to every child job through the existing cancel route. */
        RobustnessSuiteBody: {
            axes?: {
                /** @enum {string} */
                axis: "parameter_neighborhood" | "cost_stress" | "period_split" | "walk_forward" | "execution_delay" | "benchmark_comparison";
                parameter?: string;
                delta?: unknown;
                profile_id?: string;
                profile_version?: number;
                /** @example 2026-01-31 */
                train_end?: string;
                /** @example 2026-01-31 */
                validation_end?: string;
                window_sessions?: number;
                step_sessions?: number;
                delay_sessions?: number;
                benchmark_id?: string;
            }[];
            /** @description The train/validation boundary a period_split child must never read past (FR-ROB-001). */
            holdout?: {
                /** @example 2026-01-31 */
                train_end: string;
                /** @example 2026-01-31 */
                validation_end: string;
            };
        };
        Robustness: {
            /** Format: uuid */
            run_id: string;
            /** Format: uuid */
            suite_id: string;
            children: {
                /** Format: uuid */
                run_id: string;
                /** Format: uuid */
                job_id: string;
                axis: string;
                /** @enum {string} */
                status: "QUEUED" | "RUNNING" | "SUCCEEDED" | "FAILED" | "CANCELED";
            }[];
        };
        Account: {
            /** Format: uuid */
            id: string;
            /**
             * @description LIVE accounts are Phase 3 Owner-only and never creatable via this route
             * @enum {string}
             */
            account_type: "PAPER";
            name: string;
            /** @enum {string} */
            currency: "KRW";
            /** @enum {string} */
            status: "ACTIVE" | "SUSPENDED" | "CLOSED";
            /** @description The opening deposit; current cash is derived from cash_ledger, never cached here */
            initial_cash?: string | null;
            /** @enum {string} */
            cost_profile_id: "KRX_ETF_DEFAULT" | "CUSTOM";
            cost_profile_version: number;
            /** Format: date-time */
            created_at?: string;
            /** Format: date-time */
            updated_at?: string;
        };
        NewAccountBody: {
            name: string;
            /** @enum {string} */
            currency: "KRW";
            /** @example 100000000 */
            initial_cash: string;
            /**
             * @description Defaults to KRX_ETF_DEFAULT; CUSTOM is not yet configurable through this route
             * @enum {string}
             */
            cost_profile_id?: "KRX_ETF_DEFAULT" | "CUSTOM";
        };
        BindStrategy: {
            /** Format: uuid */
            account_id: string;
            /** Format: uuid */
            strategy_config_id: string;
            strategy_id: string;
            strategy_version: string;
            /** Format: date-time */
            bound_at: string;
        };
        PerformancePoint: {
            /** @example 2026-01-31 */
            trading_date: string;
            /** @example 100000000 */
            equity: string;
            /** @example 100000000 */
            cash: string;
            /** @example 100000000 */
            positions_value: string;
            /** @enum {string} */
            currency: "KRW";
            /** @description Day-over-day return, computed on read from ledger-derived equity; absent on the first point */
            return_pct?: string | null;
            /** @description Whether cash agrees with cash_ledger, the authority, as of this date */
            cash_reconciled: boolean;
        };
        Performance: {
            /** Format: uuid */
            account_id: string;
            points: components["schemas"]["PerformancePoint"][];
            /** @description Rendered verbatim; Paper results are simulated and never a guarantee of future returns */
            disclaimer: string;
        };
        Lineage: {
            /** Format: uuid */
            account_id: string;
            /** @description Immutable strategy-binding history; a rebind closes the old row and opens a new one (branching lineage) */
            bindings: {
                /** Format: uuid */
                strategy_config_id: string;
                strategy_id: string;
                strategy_version: string;
                /** Format: date-time */
                bound_at: string;
                /** Format: date-time */
                unbound_at?: string | null;
                active: boolean;
            }[];
            /** @description Each close(T) computation and the session T+1 it executed at */
            targets: {
                /** Format: uuid */
                id: string;
                /** @example 2026-01-31 */
                computed_on: string;
                /** @example 2026-01-31 */
                effective_date: string;
                /** @enum {string} */
                status: "PENDING" | "EXECUTED" | "SKIPPED";
                /** Format: date-time */
                executed_at?: string | null;
            }[];
        };
        /** @description Computed on read, never stored, so it cannot go stale against the lineage it describes. */
        Parity: {
            /** Format: uuid */
            account_id: string;
            /** @example 2026-01-31 */
            as_of: string;
            /**
             * @description NOT_COMPARABLE means the two sides came from different strategy/data/as-of inputs, so no parity claim is meaningful
             * @enum {string}
             */
            status: "MATCH" | "DIVERGENT" | "NOT_COMPARABLE";
            lineage: {
                [key: string]: unknown;
            };
            divergences: {
                [key: string]: unknown;
            }[];
            /** @description Stated on every report: backtest fills come from the NT engine, Paper fills are modeled at the next raw open plus slippage */
            fill_model_difference: string;
            /** @description True for DIVERGENT and NOT_COMPARABLE (design 15.3 grades a Paper divergence WARNING) */
            warrants_alert: boolean;
        };
        /** @description One feed row plus every attempt made to deliver it, so an outage is visible to the recipient and not only in the Owner's admin view (FR-RPT-002). */
        Notification: {
            /** Format: uuid */
            id: string;
            /** @enum {string} */
            kind: "job" | "recommendation" | "backtest" | "alert";
            title: string;
            body: string;
            /** Format: date-time */
            read_at?: string | null;
            /** Format: date-time */
            created_at: string;
            deliveries: {
                /** @enum {string} */
                channel: "web" | "email" | "admin";
                /** @enum {string} */
                status: "SUCCESS" | "FAILED";
                /** @description present only on FAILED; a recorded outage is never silent */
                error_detail?: string;
            }[];
        };
        BindStrategyBody: {
            /** Format: uuid */
            strategy_config_id: string;
        };
        Order: {
            /** Format: uuid */
            id: string;
            order_ref: string;
            instrument_id: string;
            /** @enum {string} */
            side: "BUY" | "SELL";
            /** @description decimal string (scale 4) */
            quantity: string;
            price?: string | null;
            status: string;
            /** Format: date-time */
            submitted_at?: string | null;
            /** Format: date-time */
            created_at?: string;
        };
        Position: {
            instrument_id: string;
            quantity: string;
            avg_price?: string | null;
            /** Format: date-time */
            updated_at?: string;
        };
        EquityPoint: {
            /** @example 2026-01-31 */
            trading_date: string;
            equity: string;
            cash: string;
            positions_value: string;
            currency: string;
            /** @description Whether cash agrees with cash_ledger, the authority, as of this date */
            cash_reconciled: boolean;
        };
        AdminDataset: {
            /** Format: uuid */
            id: string;
            dataset_id: string;
            version: string;
            /** @enum {string} */
            status: "READY" | "WARNING" | "BLOCKED";
            manifest_sha256?: string;
            /** Format: date-time */
            created_at?: string;
            blocking_issues?: {
                issue_code?: string;
                severity?: string;
                detail?: Record<string, never>;
            }[];
        };
        DatasetVerdict: {
            dataset_id: string;
            version: string;
            status: string;
            verdict: string;
            reason: string;
        };
        Job: {
            /** Format: uuid */
            id: string;
            job_type: string;
            /** @enum {string} */
            status: "QUEUED" | "RUNNING" | "SUCCEEDED" | "FAILED" | "CANCELED";
            priority?: number;
            idempotency_key?: string | null;
            attempt_count?: number;
            /** Format: date-time */
            created_at?: string;
            /** Format: date-time */
            started_at?: string | null;
            /** Format: date-time */
            finished_at?: string | null;
            error_code?: string | null;
            error_message?: string | null;
        };
        Worker: {
            worker_id: string;
            /** Format: date-time */
            last_heartbeat_at?: string | null;
            active_job_count: number;
        };
        AuditEntry: {
            /** Format: uuid */
            id: string;
            action: string;
            actor_role: string;
            /** Format: uuid */
            actor_user_id?: string | null;
            target_type?: string | null;
            target_id?: string | null;
            reason?: string | null;
            correlation_id?: string | null;
            /** Format: date-time */
            created_at: string;
        };
        LicensingStatus: {
            /** @example 2026-01-31 */
            as_of: string;
            datasets: {
                dataset_id: string;
                use_kind: string;
                /** @enum {string} */
                state: "PENDING" | "ACTIVE" | "EXPIRED" | "REVOKED";
                effective_from?: string | null;
                effective_until?: string | null;
                covered: boolean;
            }[];
        };
        Session: {
            /** Format: uuid */
            user_id: string;
            /** @enum {string} */
            role: "owner" | "member";
            expires_at_secs: number;
            auth_time_secs?: number;
        };
        EmptyBody: Record<string, never>;
    };
    responses: {
        /** @description 400 typed error envelope */
        Error400: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 401 typed error envelope */
        Error401: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 403 typed error envelope */
        Error403: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 404 typed error envelope */
        Error404: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 409 typed error envelope */
        Error409: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 413 typed error envelope */
        Error413: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 422 typed error envelope */
        Error422: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 429 typed error envelope */
        Error429: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 500 typed error envelope */
        Error500: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
        /** @description 501 typed error envelope */
        Error501: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorEnvelope"];
            };
        };
    };
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    get__api_v1_auth_session: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_auth_logout: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_auth_csrf: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_auth_step_up_check: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_strategies: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_strategies__strategy_id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                strategy_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_strategies__strategy_id__configs: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                strategy_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["NewStrategyConfigBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_strategy_configs: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_strategy_configs__config_id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                config_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_recommendations_runs: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Recommendation run history */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RecommendationRunPage"];
                };
            };
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_recommendations_runs: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RecommendationRunBody"];
            };
        };
        responses: {
            /** @description Recommendation run accepted */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RecommendationRun"];
                };
            };
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_recommendations_runs__run_id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                run_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Recommendation run */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RecommendationRun"];
                };
            };
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_recommendations_latest: {
        parameters: {
            query?: {
                strategy_config_id?: string;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Latest recommendation snapshot */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RecommendationLatest"];
                };
            };
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_backtests: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_backtests: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["BacktestBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_backtests__run_id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                run_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_backtests__run_id__cancel: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                run_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_backtests__run_id__metrics: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                run_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_backtests__run_id__equity: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                run_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_backtests__run_id__trades: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                run_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_backtests__run_id__robustness: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                run_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RobustnessSuiteBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_backtests_compare: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_paper_accounts: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["NewAccountBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts__account_id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_paper_accounts__account_id__bind_strategy: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["BindStrategyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts__account_id__orders: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts__account_id__positions: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts__account_id__equity: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts__account_id__performance: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts__account_id__lineage: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_paper_accounts__account_id__parity: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_datasets: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_datasets__dataset_id__approve: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                dataset_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_datasets__dataset_id__block: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                dataset_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_jobs: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_jobs__job_id__retry: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                job_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_workers: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_users: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_audit_logs: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_notifications_deliveries: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_notifications: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_notifications_subscriptions: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    put__api_v1_notifications_subscriptions: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_notifications_test: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_live_connections: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_live_connections: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_live_orders: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_live_connections__connection_id__start: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                connection_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_live_nodes__node_id__stop: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                node_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_live_kill_switch_enable: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    post__api_v1_admin_live_kill_switch_disable: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EmptyBody"];
            };
        };
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_admin_live_reconciliation: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_licensing_status: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_metrics: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_artifacts__artifact_id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
    get__api_v1_artifacts__artifact_id__download: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            400: components["responses"]["Error400"];
            401: components["responses"]["Error401"];
            403: components["responses"]["Error403"];
            404: components["responses"]["Error404"];
            409: components["responses"]["Error409"];
            413: components["responses"]["Error413"];
            422: components["responses"]["Error422"];
            429: components["responses"]["Error429"];
            500: components["responses"]["Error500"];
            501: components["responses"]["Error501"];
        };
    };
}
