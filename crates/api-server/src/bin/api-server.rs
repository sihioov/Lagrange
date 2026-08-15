//! Production API process.

use api_server::runtime::{
    build_auth_router_state, build_state, load_config, serve_with_auth, shutdown_signal,
};
use std::process::ExitCode;
use tokio::net::TcpListener;

const HELP: &str = "api-server

Serves the Lagrange Station Axum API.

Endpoints:
  /healthz              liveness (no database dependency)
  /readyz               readiness (app/admin/audit database round trips)
  /api/v1/metrics       Prometheus exposition

Configuration:
  DB_HOST + DB_PORT + DB_NAME
                         shared PostgreSQL endpoint components
  DB_USER                app-role PostgreSQL user (default app)
  DB_PASSWORD_FILE       mounted app-role password file
  ADMIN_DB_HOST + ADMIN_DB_PORT + ADMIN_DB_NAME
                         optional admin-role endpoint overrides
  ADMIN_DB_USER          admin-role PostgreSQL user (default admin)
  ADMIN_DB_PASSWORD_FILE mounted admin-role password file
  AUDIT_DB_HOST + AUDIT_DB_PORT + AUDIT_DB_NAME
                         optional audit-role endpoint overrides
  AUDIT_DB_USER          audit-role PostgreSQL user (default audit_writer)
  AUDIT_DB_PASSWORD_FILE mounted audit-role password file
  CURSOR_SECRET_FILE      mounted 32-byte cursor signing secret
  APP_ENV                 production (default), development, or test
  AUTH0_DOMAIN            HTTPS Auth0 tenant host
  AUTH0_CLIENT_ID         confidential Auth0 client id
  AUTH0_CLIENT_SECRET_FILE mounted Auth0 client-secret path
  AUTH0_REDIRECT_URI      exact HTTPS callback (optional default)
  AUTH0_AUDIENCE          optional JWT audience
  AUTH0_CLOCK_SKEW_SECS   positive JWT clock-skew allowance (optional)
  APP_LISTEN_ADDR         bind address (default 0.0.0.0:8080)
  APP_HOST + APP_PORT     alternative bind settings

Production requires component database settings, *_PASSWORD_FILE paths,
CURSOR_SECRET_FILE, and the Auth0 values above.";

fn print_help() {
    println!("{HELP}");
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if let Some(arg) = args.first() {
        eprintln!("api-server: unknown argument {arg}");
        return ExitCode::FAILURE;
    }
    let config = match load_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("api-server configuration rejected: {error}");
            return ExitCode::FAILURE;
        }
    };

    let state = match build_state(&config).await {
        Ok(state) => state,
        Err(error) => {
            // Keep connection strings and secret file contents out of the
            // process log. `build_state` reports only role labels and the
            // database driver's sanitized error text.
            eprintln!("api-server startup rejected: {error}");
            return ExitCode::FAILURE;
        }
    };

    let auth_state = match build_auth_router_state(&state) {
        Ok(auth_state) => auth_state,
        Err(error) => {
            eprintln!("api-server authentication startup rejected: {error}");
            return ExitCode::FAILURE;
        }
    };

    let listener = match TcpListener::bind(config.listen_addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("api-server cannot bind {}: {error}", config.listen_addr);
            return ExitCode::FAILURE;
        }
    };
    eprintln!("api-server listening on {}", config.listen_addr);
    if let Err(error) = serve_with_auth(listener, state, auth_state, shutdown_signal()).await {
        eprintln!("api-server stopped with error: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::HELP;

    #[test]
    fn help_documents_production_component_database_contract() {
        for key in [
            "DB_HOST + DB_PORT + DB_NAME",
            "DB_PASSWORD_FILE",
            "ADMIN_DB_HOST + ADMIN_DB_PORT + ADMIN_DB_NAME",
            "ADMIN_DB_PASSWORD_FILE",
            "AUDIT_DB_HOST + AUDIT_DB_PORT + AUDIT_DB_NAME",
            "AUDIT_DB_PASSWORD_FILE",
            "CURSOR_SECRET_FILE",
            "AUTH0_CLIENT_SECRET_FILE",
        ] {
            assert!(HELP.contains(key), "help is missing {key}");
        }
        assert!(HELP.contains("Production requires component database settings"));
        assert!(!HELP.contains("DATABASE_URL"));
    }
}
