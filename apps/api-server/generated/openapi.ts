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
    "/api/v1/recommendations/owner-beta/price-only/runs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/recommendations/owner-beta/price-only/runs */
        get: operations["get__api_v1_recommendations_owner_beta_price_only_runs"];
        put?: never;
        /** POST /api/v1/recommendations/owner-beta/price-only/runs */
        post: operations["post__api_v1_recommendations_owner_beta_price_only_runs"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/recommendations/owner-beta/price-only/runs/{run_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/recommendations/owner-beta/price-only/runs/{run_id} */
        get: operations["get__api_v1_recommendations_owner_beta_price_only_runs__run_id_"];
        put?: never;
        post?: never;
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
    "/api/v1/candidates/feed/latest": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/candidates/feed/latest */
        get: operations["get__api_v1_candidates_feed_latest"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/candidates/feed/{date}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/candidates/feed/{date} */
        get: operations["get__api_v1_candidates_feed__date_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/stocks/{instrument_id}/analysis": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/stocks/{instrument_id}/analysis */
        get: operations["get__api_v1_stocks__instrument_id__analysis"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/screener/query": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/screener/query */
        post: operations["post__api_v1_screener_query"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/screener/screens": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/screener/screens */
        get: operations["get__api_v1_screener_screens"];
        put?: never;
        /** POST /api/v1/screener/screens */
        post: operations["post__api_v1_screener_screens"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/screener/screens/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/screener/screens/{id} */
        get: operations["get__api_v1_screener_screens__id_"];
        /** PUT /api/v1/screener/screens/{id} */
        put: operations["put__api_v1_screener_screens__id_"];
        post?: never;
        /** DELETE /api/v1/screener/screens/{id} */
        delete: operations["delete__api_v1_screener_screens__id_"];
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
    "/api/v1/paper/accounts/{account_id}/recommendation-previews": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/paper/accounts/{account_id}/recommendation-previews */
        post: operations["post__api_v1_paper_accounts__account_id__recommendation_previews"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** GET /api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id} */
        get: operations["get__api_v1_paper_accounts__account_id__recommendation_previews__preview_id_"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /** POST /api/v1/paper/accounts/{account_id}/recommendation-previews/{preview_id}/apply */
        post: operations["post__api_v1_paper_accounts__account_id__recommendation_previews__preview_id__apply"];
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
        ErrorCode: "SESSION_UNKNOWN" | "SESSION_EXPIRED" | "FORBIDDEN" | "DATA_ENTITLEMENT_REQUIRED" | "OWNER_ONLY_DEVELOPMENT_PATH" | "CSRF_DENIED" | "STEP_UP_NOT_OWNER" | "STEP_UP_MFA_REQUIRED" | "STEP_UP_AUTH_TIME_ABSENT" | "STEP_UP_AUTH_TIME_STALE" | "RESOURCE_NOT_FOUND" | "INVALID_PARAMETER" | "INVALID_DATE" | "INVALID_DECIMAL" | "INVALID_CURSOR" | "IDEMPOTENCY_KEY_REQUIRED" | "IDEMPOTENCY_KEY_MISMATCH" | "DUPLICATE_RESOURCE" | "PAYLOAD_TOO_LARGE" | "DATASET_BLOCKED" | "DATA_STALE" | "INVALID_STRATEGY_PARAMETER" | "UNSUPPORTED_MARKET_CURRENCY" | "BACKTEST_CAPACITY_EXCEEDED" | "ROBUSTNESS_CAPACITY_EXCEEDED" | "RECOMMENDATION_CAPACITY_EXCEEDED" | "OWNER_BETA_PRICE_INPUT_UNAVAILABLE" | "OWNER_BETA_STRATEGY_UNSUPPORTED" | "REBALANCE_PREVIEW_CAPACITY_EXCEEDED" | "REBALANCE_PREVIEW_BINDING_REQUIRED" | "REBALANCE_PREVIEW_NOT_READY" | "REBALANCE_PREVIEW_DATA_BLOCKED" | "REBALANCE_PREVIEW_ENTITLEMENT_REQUIRED" | "REBALANCE_PREVIEW_STALE" | "REBALANCE_PREVIEW_FAILED" | "REBALANCE_PREVIEW_CONFLICT" | "RESULT_INTEGRITY_FAILED" | "LIVE_RECONCILIATION_REQUIRED" | "LIVE_KILL_SWITCH_ENGAGED" | "LIVE_CONNECTION_NOT_CONFIGURED" | "RISK_LIMIT_EXCEEDED" | "ORDER_STATE_UNKNOWN" | "NOT_IMPLEMENTED" | "INTERNAL";
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
        OwnerBetaPriceOnlyRun: {
            /** Format: uuid */
            run_id: string;
            /** Format: uuid */
            job_id: string;
            /** @enum {string} */
            status: "PENDING";
        };
        OwnerBetaPriceOnlyRunBody: {
            /** Format: uuid */
            strategy_config_id: string;
            /** @example 2026-01-31 */
            as_of: string;
        };
        OwnerBetaPriceOnlyReadItem: {
            /** @enum {string} */
            instrument_id: "069500.KRX" | "102110.KRX" | "114260.KRX" | "132030.KRX" | "138230.KRX" | "152100.KRX" | "148020.KRX" | "305720.KRX" | "278530.KRX" | "292150.KRX" | "360750.KRX";
            rank?: number | null;
            target_weight?: string | null;
            excluded: boolean;
            /** @enum {string|null} */
            exclusion_reason?: "SELECTED_TOP_N" | "NOT_SELECTED_BEYOND_TOP_N" | "EXCLUDED_MANDATORY_FACTOR_NULL" | "ALL_CASH_NO_ELIGIBLE" | "WEIGHT_CAPPED_AT_MAX" | "WEIGHT_ROUNDING_RESIDUE_TO_CASH" | "CASH_FLOOR_APPLIED" | "BENCHMARK_HELD" | "TREND_POSITIVE" | "TREND_NEGATIVE_CASH" | "ABSOLUTE_MOMENTUM_PASSED" | "DEFENSIVE_CASH_SELECTED" | "INVERSE_VOL_WEIGHTED" | "NOT_SELECTED_BY_STRATEGY" | null;
            reason_codes: ("SELECTED_TOP_N" | "NOT_SELECTED_BEYOND_TOP_N" | "EXCLUDED_MANDATORY_FACTOR_NULL" | "ALL_CASH_NO_ELIGIBLE" | "WEIGHT_CAPPED_AT_MAX" | "WEIGHT_ROUNDING_RESIDUE_TO_CASH" | "CASH_FLOOR_APPLIED" | "BENCHMARK_HELD" | "TREND_POSITIVE" | "TREND_NEGATIVE_CASH" | "ABSOLUTE_MOMENTUM_PASSED" | "DEFENSIVE_CASH_SELECTED" | "INVERSE_VOL_WEIGHTED" | "NOT_SELECTED_BY_STRATEGY")[];
            factors: {
                [key: string]: string;
            };
        };
        OwnerBetaPriceOnlyReadRun: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            job_id: string;
            /** Format: uuid */
            strategy_config_id: string;
            strategy_id: string;
            strategy_version: string;
            /** @example 2026-01-31 */
            as_of: string;
            /** @enum {string} */
            status: "PENDING" | "RUNNING" | "SUCCEEDED" | "FAILED" | "CANCELED";
            /** @constant */
            input_kind: "owner_beta_historical_price_only_v1";
            /** @constant */
            capability: "PRICE_RETURN_ONLY";
            /** @constant */
            audience: "OWNER_ONLY";
            /** @constant */
            vendor_snapshot: true;
            /** @constant */
            strict_pit: false;
            strategy_config_sha256: string;
            candidate_content_sha256: string;
            artifact_manifest_sha256: string;
            stage5_manifest_sha256: string;
            action_manifest_sha256: string;
            approval_registry_sha256: string;
            factor_snapshot_sha256?: string | null;
            target_snapshot_sha256?: string | null;
            cash_weight?: string | null;
            error_code?: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            started_at?: string | null;
            /** Format: date-time */
            finished_at?: string | null;
            /** Format: date-time */
            updated_at: string;
            items: components["schemas"]["OwnerBetaPriceOnlyReadItem"][];
        };
        OwnerBetaPriceOnlyReadListItem: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            job_id: string;
            /** Format: uuid */
            strategy_config_id: string;
            strategy_id: string;
            strategy_version: string;
            /** @example 2026-01-31 */
            as_of: string;
            /** @enum {string} */
            status: "PENDING" | "RUNNING" | "SUCCEEDED" | "FAILED" | "CANCELED";
            /** @constant */
            input_kind: "owner_beta_historical_price_only_v1";
            /** @constant */
            capability: "PRICE_RETURN_ONLY";
            /** @constant */
            audience: "OWNER_ONLY";
            /** @constant */
            vendor_snapshot: true;
            /** @constant */
            strict_pit: false;
            strategy_config_sha256: string;
            candidate_content_sha256: string;
            artifact_manifest_sha256: string;
            stage5_manifest_sha256: string;
            action_manifest_sha256: string;
            approval_registry_sha256: string;
            factor_snapshot_sha256?: string | null;
            target_snapshot_sha256?: string | null;
            cash_weight?: string | null;
            error_code?: string | null;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            started_at?: string | null;
            /** Format: date-time */
            finished_at?: string | null;
            /** Format: date-time */
            updated_at: string;
        };
        OwnerBetaPriceOnlyReadPage: {
            items: components["schemas"]["OwnerBetaPriceOnlyReadListItem"][];
            /** @description opaque signed cursor; null when the last page */
            next_cursor: string | null;
            has_more: boolean;
        };
        CandidateDatasetPins: {
            /** Format: uuid */
            universe_snapshot_id: string;
            price: {
                /** Format: uuid */
                dataset_version_id: string;
                curated_version: number;
                manifest_sha256: string;
            };
            market_status: components["schemas"]["CandidateSourcePin"];
            flow: components["schemas"]["CandidateSourcePin"];
            fundamental: components["schemas"]["CandidateSourcePin"];
            /** Format: uuid */
            sector_version_id: string;
            input_identity_sha256: string;
        };
        /**
         * @description Point-in-time candidate universe; omitted API queries default to kospi200.
         * @enum {string}
         */
        UniverseKey: "kospi200" | "kosdaq150";
        CandidateSourcePin: {
            /** Format: uuid */
            dataset_version_id: string;
            manifest_sha256: string;
        };
        CandidateScores: {
            flow: number | null;
            fundamental: number | null;
            technical: number | null;
            total: number | null;
        };
        CandidateCoverage: {
            flow: number;
            fundamental: number;
            technical: number;
        };
        CandidateAnalysis: {
            /** Format: uuid */
            analysis_id: string;
            /** Format: uuid */
            run_id: string;
            universe: components["schemas"]["UniverseKey"];
            /** @example 005930.KRX */
            instrument_id: string;
            name?: string | null;
            sector_code: string;
            /**
             * @description Versioned fundamental scoring profile selected for the instrument.
             * @enum {string}
             */
            fundamental_profile: "candidate-non-financial-v1" | "candidate-financial-v1" | "unsupported";
            eligible: boolean;
            exclusion_codes: string[];
            scores: components["schemas"]["CandidateScores"];
            coverage: components["schemas"]["CandidateCoverage"];
            /** @enum {string} */
            evidence_strength: "STRONG" | "MODERATE" | "WEAK";
            rank?: number | null;
            /** @enum {string} */
            normalization_scope: "SECTOR" | "UNIVERSE_FALLBACK" | "UNAVAILABLE";
            factors: {
                [key: string]: unknown;
            };
            /** @description Deterministic upside/neutral/downside trigger records; never probabilities or target prices. */
            scenarios: {
                [key: string]: unknown;
            };
            provenance: {
                [key: string]: unknown;
            };
            content_sha256: string;
        };
        CandidateResearchEnvelope: {
            universe?: components["schemas"]["UniverseKey"] | null;
            /** @enum {string} */
            state: "READY" | "STALE";
            /** @example 2026-01-31 */
            as_of: string;
            /** Format: date-time */
            cutoff_at: string;
            scoring_config: {
                version: string;
                sha256: string;
            };
            dataset_pins: components["schemas"]["CandidateDatasetPins"];
            license_attributions: components["schemas"]["CandidateLicenseAttribution"][];
            disclaimer: string;
        };
        CandidateLicenseAttribution: {
            /** @enum {string} */
            source: "price" | "universe" | "market_status" | "flow" | "fundamental" | "sector";
            dataset_id: string;
            license_ref: string;
            /** Format: uuid */
            entitlement_id: string;
            contract_reference: string;
            contract_document_sha256: string;
        };
        CandidateFeed: components["schemas"]["CandidateResearchEnvelope"] & {
            /** Format: uuid */
            feed_id: string;
            universe: components["schemas"]["UniverseKey"];
            /** Format: date-time */
            published_at: string;
            computation_seq: number;
            items: components["schemas"]["CandidateAnalysis"][];
        };
        StockAnalysisResponse: components["schemas"]["CandidateResearchEnvelope"] & {
            universe: components["schemas"]["UniverseKey"];
            analysis: components["schemas"]["CandidateAnalysis"];
        };
        ScreenCriteria: {
            /** @description One or both universes; omitted defaults to kospi200. */
            universes?: components["schemas"]["UniverseKey"][];
            sectors?: string[];
            evidence_strength?: ("STRONG" | "MODERATE" | "WEAK")[];
            min_total_score?: number | null;
            min_flow_score?: number | null;
            min_fundamental_score?: number | null;
            min_technical_score?: number | null;
        };
        ScreenerQueryBody: {
            /**
             * Format: uuid
             * @description Legacy exact single-universe run pin; incompatible with both universes.
             */
            run_id?: string | null;
            /** @example 2026-01-31 */
            as_of?: string;
            criteria: components["schemas"]["ScreenCriteria"];
            /** @description opaque HMAC-signed frozen run-set/universe/decimal-score/instrument cursor (v2; legacy v1 is KOSPI-only) */
            cursor?: string | null;
            /** @default 25 */
            limit: number | null;
        };
        ScreenerResult: components["schemas"]["CandidateResearchEnvelope"] & {
            universe?: components["schemas"]["UniverseKey"] | null;
            universes: components["schemas"]["UniverseKey"][];
            /** Format: uuid */
            run_id?: string | null;
            run_ids: {
                universe: components["schemas"]["UniverseKey"];
                /** Format: uuid */
                run_id: string;
            }[];
            items: components["schemas"]["CandidateAnalysis"][];
            next_cursor: string | null;
        };
        SavedScreenBody: {
            name: string;
            criteria: components["schemas"]["ScreenCriteria"];
        };
        SavedScreen: {
            /** Format: uuid */
            id: string;
            name: string;
            /** @enum {integer} */
            criteria_schema_version: 1 | 2;
            criteria: components["schemas"]["ScreenCriteria"];
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            updated_at: string;
        };
        SavedScreenList: {
            items: components["schemas"]["SavedScreen"][];
        };
        DeleteSavedScreenResult: {
            /** Format: uuid */
            id: string;
            /** @constant */
            deleted: true;
        };
        BacktestRun: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            owner_user_id: string;
            /** @description Whether the current actor may mutate or cancel this run */
            can_manage: boolean;
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
            /** Format: uuid */
            owner_user_id: string;
            /** @description Whether the current actor may change this account */
            can_manage: boolean;
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
        RebalancePreviewBody: {
            /** Format: uuid */
            recommendation_run_id: string;
        };
        ApplyRebalancePreviewBody: {
            preview_token: string;
        };
        RebalancePreviewLineage: {
            /** Format: uuid */
            account_id: string;
            /** Format: uuid */
            recommendation_run_id: string;
            /** Format: uuid */
            target_portfolio_id: string;
            /** Format: uuid */
            strategy_config_id: string;
            /** Format: uuid */
            dataset_version_id: string;
            curated_version: number;
            dataset_manifest_sha256: string;
            /** Format: int64 */
            account_state_version: number;
            account_state_sha256: string;
            target_portfolio_sha256: string;
        };
        RebalancePreviewDecision: {
            /** @example 069500.KRX */
            instrument_id: string;
            /** @example 100000000 */
            current_quantity: string;
            /** @example 100000000 */
            current_value: string;
            /** @example 100000000 */
            current_weight: string;
            /** @example 100000000 */
            target_value: string;
            /** @example 100000000 */
            target_weight: string;
            /** @example 100000000 */
            delta_value: string;
            /** @enum {string} */
            action: "BUY" | "SELL" | "SKIP";
            /** @enum {string|null} */
            skip_reason: "BELOW_REBALANCE_THRESHOLD" | "BELOW_MIN_TRADE" | "NO_AVAILABLE_CASH" | "NO_AFFORDABLE_LOT" | null;
        };
        RebalancePreviewOrder: {
            /** @example 069500.KRX */
            instrument_id: string;
            /** @enum {string} */
            side: "BUY" | "SELL";
            /** @example 100000000 */
            quantity: string;
            /** @example 100000000 */
            raw_price: string;
            /** @example 100000000 */
            estimated_execution_price: string;
            /** @example 100000000 */
            notional: string;
            /** @example 100000000 */
            commission: string;
            /** @example 100000000 */
            tax: string;
            /** @example 100000000 */
            informational_slippage: string;
        };
        RebalancePreviewResult: {
            /** @constant */
            schema_version: 1;
            /** @constant */
            price_basis: "RECOMMENDATION_CLOSE";
            /** @example 2026-01-31 */
            price_date: string;
            /** @example 2026-01-31 */
            proposed_effective_date: string;
            /** @example 100000000 */
            equity: string;
            /** @example 100000000 */
            cash_before: string;
            /** @example 100000000 */
            available_cash: string;
            /** @example 100000000 */
            leftover_cash: string;
            /** @example 100000000 */
            buy_notional: string;
            /** @example 100000000 */
            sell_notional: string;
            /** @example 100000000 */
            explicit_fees: string;
            /** @example 100000000 */
            informational_slippage: string;
            decisions: components["schemas"]["RebalancePreviewDecision"][];
            orders: components["schemas"]["RebalancePreviewOrder"][];
            /** @constant */
            warning_code: "INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED";
            lineage: components["schemas"]["RebalancePreviewLineage"];
        };
        RebalancePreviewError: {
            code: string;
            message: string;
        };
        RebalancePreview: {
            /** Format: uuid */
            id: string;
            /** Format: uuid */
            account_id: string;
            /** Format: uuid */
            recommendation_run_id: string;
            /** Format: uuid */
            target_portfolio_id: string;
            /** Format: uuid */
            strategy_config_id: string;
            /** Format: uuid */
            job_id: string;
            /** @enum {string} */
            status: "PENDING" | "RUNNING" | "READY" | "FAILED" | "APPLIED";
            /** @constant */
            price_basis: "RECOMMENDATION_CLOSE";
            /** @example 2026-01-31 */
            price_date: string;
            proposed_effective_date: string | null;
            /** Format: uuid */
            dataset_version_id: string;
            dataset_manifest_sha256: string;
            target_portfolio_sha256: string;
            preview_token: string | null;
            result?: components["schemas"]["RebalancePreviewResult"];
            error?: components["schemas"]["RebalancePreviewError"];
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            started_at: string | null;
            /** Format: date-time */
            completed_at: string | null;
            /** Format: date-time */
            applied_at: string | null;
            /** Format: date-time */
            updated_at: string;
        };
        AppliedRebalancePreview: {
            /** Format: uuid */
            preview_id: string;
            /** Format: uuid */
            pending_target_id: string;
            /** @example 2026-01-31 */
            effective_date: string;
            /** @constant */
            source_kind: "MANUAL_RECOMMENDATION";
            /** @constant */
            status: "APPLIED";
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
            /** @enum {string} */
            owner_beta_access_mode: "disabled" | "owner_only";
            /** @enum {string} */
            owner_beta_paper_mode: "disabled" | "enabled";
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
        /** @description 503 typed error envelope */
        Error503: {
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_recommendations_owner_beta_price_only_runs: {
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
            /** @description Owner-beta price-only recommendation history */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OwnerBetaPriceOnlyReadPage"];
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
            503: components["responses"]["Error503"];
        };
    };
    post__api_v1_recommendations_owner_beta_price_only_runs: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["OwnerBetaPriceOnlyRunBody"];
            };
        };
        responses: {
            /** @description Owner-beta price-only recommendation accepted */
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OwnerBetaPriceOnlyRun"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_recommendations_owner_beta_price_only_runs__run_id_: {
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
            /** @description Owner-beta price-only recommendation */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OwnerBetaPriceOnlyReadRun"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_candidates_feed_latest: {
        parameters: {
            query?: {
                universe?: components["schemas"]["UniverseKey"];
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Immutable daily stock-research Top-5 feed */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CandidateFeed"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_candidates_feed__date_: {
        parameters: {
            query?: {
                universe?: components["schemas"]["UniverseKey"];
            };
            header?: never;
            path: {
                date: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Immutable daily stock-research Top-5 feed */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CandidateFeed"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_stocks__instrument_id__analysis: {
        parameters: {
            query?: {
                date?: string;
                universe?: components["schemas"]["UniverseKey"];
            };
            header?: never;
            path: {
                instrument_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Point-in-time deep stock analysis */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["StockAnalysisResponse"];
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
            503: components["responses"]["Error503"];
        };
    };
    post__api_v1_screener_query: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ScreenerQueryBody"];
            };
        };
        responses: {
            /** @description Point-in-time candidate screen result */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ScreenerResult"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_screener_screens: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Actor-owned saved screens */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SavedScreenList"];
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
            503: components["responses"]["Error503"];
        };
    };
    post__api_v1_screener_screens: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SavedScreenBody"];
            };
        };
        responses: {
            /** @description Saved screen created */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SavedScreen"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_screener_screens__id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Actor-owned saved screen */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SavedScreen"];
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
            503: components["responses"]["Error503"];
        };
    };
    put__api_v1_screener_screens__id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SavedScreenBody"];
            };
        };
        responses: {
            /** @description Actor-owned saved screen */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SavedScreen"];
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
            503: components["responses"]["Error503"];
        };
    };
    delete__api_v1_screener_screens__id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Saved screen deleted */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DeleteSavedScreenResult"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
        };
    };
    post__api_v1_paper_accounts__account_id__recommendation_previews: {
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
                "application/json": components["schemas"]["RebalancePreviewBody"];
            };
        };
        responses: {
            /** @description Existing rebalance preview replayed */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RebalancePreview"];
                };
            };
            /** @description Rebalance preview accepted */
            202: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RebalancePreview"];
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
            503: components["responses"]["Error503"];
        };
    };
    get__api_v1_paper_accounts__account_id__recommendation_previews__preview_id_: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
                preview_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Paper rebalance preview */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RebalancePreview"];
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
            503: components["responses"]["Error503"];
        };
    };
    post__api_v1_paper_accounts__account_id__recommendation_previews__preview_id__apply: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                account_id: string;
                preview_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ApplyRebalancePreviewBody"];
            };
        };
        responses: {
            /** @description Rebalance preview queued for Paper execution */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AppliedRebalancePreview"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
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
            503: components["responses"]["Error503"];
        };
    };
}
