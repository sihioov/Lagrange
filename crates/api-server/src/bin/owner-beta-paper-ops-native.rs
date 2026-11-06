use std::io::Write;

use job_queue::owner_beta::paper::operations::{ResultWire, Verb, run_native};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args_os();
    let _program = args.next();
    let verb_arg = args.next();
    if args.next().is_some() {
        invalid_request();
    }
    let Some(verb_arg) = verb_arg.and_then(|arg| arg.into_string().ok()) else {
        invalid_request();
    };
    let Some(verb) = Verb::parse(&verb_arg) else {
        invalid_request();
    };
    let result = run_native(verb).await;
    let exit_code = if write_result(&result) {
        result.exit_code()
    } else {
        // The caller cannot infer a rollback when the response channel failed.
        4
    };
    std::process::exit(exit_code);
}

fn write_result(result: &ResultWire) -> bool {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&result.encode()).is_ok()
        && stdout.write_all(b"\n").is_ok()
        && stdout.flush().is_ok()
}

fn invalid_request() -> ! {
    // The closed result schema requires a recognized verb. Invalid invocations
    // get a fixed, non-diagnostic error on stderr and exit 2.
    let _ = writeln!(std::io::stderr().lock(), "WIRE_INVALID");
    std::process::exit(2)
}
