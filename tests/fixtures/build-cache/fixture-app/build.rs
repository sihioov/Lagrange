use std::{env, fs, path::PathBuf};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing fixture build environment: {name}"))
}

fn main() {
    let setting = required("CACHE_FIXTURE_BUILD_SETTING");
    let commit = required("CACHE_FIXTURE_COMMIT");
    let embedded = fs::read_to_string("data/embedded.txt")
        .expect("build-cache fixture embedded data must be readable");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let generated = format!(
        "generated-v1|commit={commit}|setting={setting}|embedded={}",
        embedded.trim()
    );
    fs::write(out_dir.join("generated.txt"), generated)
        .expect("build-cache fixture generated data must be writable");

    println!("cargo:rerun-if-changed=data/embedded.txt");
    println!("cargo:rerun-if-env-changed=CACHE_FIXTURE_BUILD_SETTING");
    println!("cargo:rerun-if-env-changed=CACHE_FIXTURE_COMMIT");
    println!("cargo:rustc-env=CACHE_FIXTURE_COMMIT={commit}");
    println!("cargo:rustc-env=CACHE_FIXTURE_BUILD_SETTING={setting}");
}
