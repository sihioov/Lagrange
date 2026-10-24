//! Opt-in G3 proof driver for the authenticated approved-simulation path.
//!
//! The separately delivered runner is started by the coordinator only after
//! this program writes `ready.json`; this example never launches child work.

// The shared harness already declares its own dead_code allowance.

#[path = "../tests/common/mod.rs"]
mod common;
mod simulation_connection_proof;

#[tokio::main]
async fn main() {
    std::panic::set_hook(Box::new(|_| eprintln!("proof:fail:panic")));
    if let Err(failure) = simulation_connection_proof::run(std::env::args_os().skip(1)).await {
        eprintln!("{}", failure.label());
        std::process::exit(1);
    }
}
