use apollo_federation::composition;
use apollo_federation::error::CompositionError;
use apollo_federation::subgraph::typestate::Subgraph;

fn sdl(s: &str) -> String {
    s.trim().to_string()
}

#[test]
fn compose_pipeline_happy_path() {
    let s1 = Subgraph::parse(
        "A",
        "http://a",
        &sdl(
            r#"
            type Query { t: T }
            type T @key(fields: "id") { id: ID }
            "#,
        ),
    )
    .expect("valid subgraph A");

    let s2 = Subgraph::parse(
        "B",
        "http://b",
        &sdl(
            r#"
            type T @key(fields: "id") { id: ID, x: Int }
            "#,
        ),
    )
    .expect("valid subgraph B");

    let result = composition::compose(vec![s1, s2]);
    assert!(result.is_ok(), "expected compose to succeed, got: {:?}", result);

    let supergraph = result.unwrap();
    // Ensure we can derive API schema from the composed supergraph.
    let api = supergraph.to_api_schema(Default::default()).expect("api schema");
    assert!(api.schema().get_object("T").is_some());
}

#[test]
fn compose_pipeline_error_on_non_shareable_field_sharing() {
    // Both subgraphs define the same non-shareable field `a` on `T` → should fail composition.
    let s1 = Subgraph::parse(
        "A",
        "http://a",
        &sdl(
            r#"
            type Query { t: T }
            type T @key(fields: "id") { id: ID, a: Int }
            "#,
        ),
    )
    .expect("valid subgraph A");

    let s2 = Subgraph::parse(
        "B",
        "http://b",
        &sdl(
            r#"
            type T @key(fields: "id") { id: ID, a: Int }
            "#,
        ),
    )
    .expect("valid subgraph B");

    let result = composition::compose(vec![s1, s2]);
    assert!(result.is_err(), "expected compose to error");
    let errors: Vec<CompositionError> = result.err().unwrap();
    assert!(!errors.is_empty());
}

#[test]
fn compose_pipeline_satisfiability_failure() {
    // Force a satisfiability issue by requiring a field that no subgraph provides.
    let s1 = Subgraph::parse(
        "A",
        "http://a",
        &sdl(
            r#"
            type Query { t: T }
            type T @key(fields: "id") { id: ID, x: Int @requires(fields: "y") }
            "#,
        ),
    )
    .expect("valid subgraph A");

    let s2 = Subgraph::parse(
        "B",
        "http://b",
        &sdl(
            r#"
            type T @key(fields: "id") { id: ID }
            "#,
        ),
    )
    .expect("valid subgraph B");

    let result = composition::compose(vec![s1, s2]);
    assert!(result.is_err(), "expected satisfiability validation to fail");
}

#[test]
fn compose_pipeline_emits_hints_on_success() {
    let s1 = Subgraph::parse(
        "A",
        "http://a",
        &sdl(
            r#"
            type Query { t: T }
            type T @key(fields: "id") { id: ID }
            "#,
        ),
    )
    .expect("valid subgraph A");

    let s2 = Subgraph::parse(
        "B",
        "http://b",
        &sdl(
            r#"
            type T @key(fields: "id") { id: ID }
            "#,
        ),
    )
    .expect("valid subgraph B");

    let supergraph = composition::compose(vec![s1, s2]).expect("compose ok");
    // Hints may be empty, but ensure the accessor works
    let _ = supergraph.hints();
} 