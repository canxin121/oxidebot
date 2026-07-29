#[test]
fn macros_expand_against_the_runtime_contract() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/renamed_dependency.rs");
}
