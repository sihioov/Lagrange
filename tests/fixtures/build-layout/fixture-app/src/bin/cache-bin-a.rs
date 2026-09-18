const GENERATED: &str = include_str!(concat!(env!("OUT_DIR"), "/generated.txt"));
const EMBEDDED: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/embedded.txt"));
const SOURCE_MARKER: &str = "source-v1";

fn main() {
    println!(
        "cache-bin-a|commit={}|setting={}|source={}|embedded={}|{}|shared={}",
        env!("CACHE_FIXTURE_COMMIT"),
        env!("CACHE_FIXTURE_BUILD_SETTING"),
        SOURCE_MARKER,
        EMBEDDED.trim(),
        GENERATED.trim(),
        build_cache_fixture_lib::shared_number()
    );
}
