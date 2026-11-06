use job_queue::owner_beta::paper::operations::Verb;

#[test]
fn native_bridge_exposes_only_the_adopted_verbs() {
    for verb in [
        "configure",
        "inspect",
        "enable",
        "begin",
        "reconcile",
        "off",
        "restore-off",
        "recover",
    ] {
        assert!(Verb::parse(verb).is_some(), "accepted verb {verb}");
    }
    for invalid in [
        "",
        "help",
        "--help",
        "inspect --format json",
        "enable/../begin",
        "Configure",
    ] {
        assert!(Verb::parse(invalid).is_none(), "rejected input {invalid:?}");
    }
}
