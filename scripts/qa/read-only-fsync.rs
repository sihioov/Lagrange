//! Linux smoke probe for the Raw evidence durability contract.
//!
//! This file is compiled directly by the QA smoke and is deliberately outside
//! the research-worker Docker build context. The resulting binary is transient.

use std::env;
use std::fs::File;

fn main() {
    let path = env::args_os()
        .nth(1)
        .expect("usage: read-only-fsync <path>");
    let file = File::open(&path).expect("open evidence read-only");
    file.sync_all().expect("fsync read-only evidence");
}
