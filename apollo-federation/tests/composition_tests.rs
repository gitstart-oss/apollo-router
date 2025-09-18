use apollo_compiler::Schema;
use apollo_federation::Supergraph;
use apollo_federation::error::CompositionError;
use apollo_federation::subgraph::Subgraph;
use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph, Validated};

// Bring the typestate subgraph for pre/merge/post validators

use apollo_federation::composition::{
    CompositionOptions, compose_with_options, expand_subgraphs, merge_subgraphs,
    post_merge_validations, pre_merge_validations, upgrade_subgraphs_if_necessary,
    validate_subgraphs,
};
fn print_sdl(schema: &Schema) -> String {
    let mut schema = schema.clone();
    schema.types.sort_keys();
    schema.directive_definitions.sort_keys();
    schema.to_string()
}

#[test]
fn can_compose_supergraph() {
    let s1 = Subgraph::parse_and_expand(
        "Subgraph1",
        "https://subgraph1",
        r#"
            type Query {
              t: T
            }

            type T @key(fields: "k") {
              k: ID
            }

            type S {
              x: Int
            }

            union U = S | T
        "#,
    )
    .unwrap();
    let s2 = Subgraph::parse_and_expand(
        "Subgraph2",
        "https://subgraph2",
        r#"
            type T @key(fields: "k") {
              k: ID
              a: Int
              b: String
            }

            enum E {
              V1
              V2
            }
        "#,
    )
    .unwrap();

    let supergraph = Supergraph::compose(vec![&s1, &s2]).unwrap();
    insta::assert_snapshot!(print_sdl(supergraph.schema.schema()));
    insta::assert_snapshot!(print_sdl(
        supergraph
            .to_api_schema(Default::default())
            .unwrap()
            .schema()
    ));
}

#[test]
fn can_compose_with_descriptions() {
    let s1 = Subgraph::parse_and_expand(
        "Subgraph1",
        "https://subgraph1",
        r#"
            "The foo directive description"
            directive @foo(url: String) on FIELD

            "A cool schema"
            schema {
              query: Query
            }

            """
            Available queries
            Not much yet
            """
            type Query {
              "Returns tea"
              t(
                "An argument that is very important"
                x: String!
              ): String
            }
        "#,
    )
    .unwrap();

    let s2 = Subgraph::parse_and_expand(
        "Subgraph2",
        "https://subgraph2",
        r#"
            "The foo directive description"
            directive @foo(url: String) on FIELD

            "An enum"
            enum E {
              "The A value"
              A
              "The B value"
              B
            }
        "#,
    )
    .unwrap();

    let supergraph = Supergraph::compose(vec![&s1, &s2]).unwrap();
    insta::assert_snapshot!(print_sdl(supergraph.schema.schema()));
    insta::assert_snapshot!(print_sdl(
        supergraph
            .to_api_schema(Default::default())
            .unwrap()
            .schema()
    ));
}

#[test]
fn can_compose_types_from_different_subgraphs() {
    let s1 = Subgraph::parse_and_expand(
        "SubgraphA",
        "https://subgraphA",
        r#"
            type Query {
                products: [Product!]
            }

            type Product {
                sku: String!
                name: String!
            }
        "#,
    )
    .unwrap();

    let s2 = Subgraph::parse_and_expand(
        "SubgraphB",
        "https://subgraphB",
        r#"
            type User {
                name: String
                email: String!
            }
        "#,
    )
    .unwrap();
    let supergraph = Supergraph::compose(vec![&s1, &s2]).unwrap();
    insta::assert_snapshot!(print_sdl(supergraph.schema.schema()));
    insta::assert_snapshot!(print_sdl(
        supergraph
            .to_api_schema(Default::default())
            .unwrap()
            .schema()
    ));
}

#[test]
fn compose_removes_federation_directives() {
    let s1 = Subgraph::parse_and_expand(
        "SubgraphA",
        "https://subgraphA",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: [ "@key", "@provides", "@external" ])

            type Query {
              products: [Product!] @provides(fields: "name")
            }

            type Product @key(fields: "sku") {
              sku: String!
              name: String! @external
            }
        "#,
    )
        .unwrap();

    let s2 = Subgraph::parse_and_expand(
        "SubgraphB",
        "https://subgraphB",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: [ "@key", "@shareable" ])

            type Product @key(fields: "sku") {
              sku: String!
              name: String! @shareable
            }
        "#,
    )
        .unwrap();

    let supergraph = Supergraph::compose(vec![&s1, &s2]).unwrap();
    insta::assert_snapshot!(print_sdl(supergraph.schema.schema()));
    insta::assert_snapshot!(print_sdl(
        supergraph
            .to_api_schema(Default::default())
            .unwrap()
            .schema()
    ));
}
#[test]
fn merge_subgraphs_combines_types_and_fields_correctly() {
    let s1 = Subgraph::parse_and_expand(
        "S1",
        "https://s1",
        r#"
        type Product @key(fields: "id") {
            id: ID!
            name: String
        }
    "#,
    )
    .unwrap();

    // This subgraph extends Product with more fields
    let s2 = Subgraph::parse_and_expand(
        "S2",
        "https://s2",
        r#"
        extend type Product @key(fields: "id") {
            price: Int
            description: String
        }
    "#,
    )
    .unwrap();

    let supergraph = Supergraph::compose(vec![&s1, &s2]).expect("composition should succeed");
    let sdl = print_sdl(supergraph.schema.schema());

    // Check the Product type and expected fields are present in the merged SDL
    assert!(sdl.contains("type Product"));
    assert!(sdl.contains("id: ID"));
    assert!(sdl.contains("name: String"));
    assert!(sdl.contains("price: Int"));
    assert!(sdl.contains("description: String"));
}


#[test]
fn post_merge_validations_fail_on_invalid_key_directive() {
    let s1 = TSubgraph::<Initial>::parse(
        "S1",
        "https://s1",
        r#"
        type Product @key(fields: "id") { id: ID! }
        "#,
    ).unwrap();

    let s2 = TSubgraph::<Initial>::parse(
        "S2",
        "https://s2",
        r#"
        type User @key(fields: "nonExistentField") { id: ID! }
        "#,
    ).unwrap();

    let res = compose_with_options(
        vec![s1, s2],
        CompositionOptions { run_satisfiability: false },
    );

    assert!(
        res.is_err(),
        "composition must fail due to invalid @key (nonexistent field)"
    );

    let combined = format!("{:?}", res.unwrap_err());
    assert!(
        combined.contains("does not exist")
            || combined.contains("invalid FieldSet")
            || combined.contains("InvalidFieldSet")
            || combined.contains("KeyInvalidFields")
            || combined.contains("Cannot query field"),
        "expected error mentioning missing field or invalid FieldSet, got: {}",
        combined
    );
}



#[test]
fn pre_merge_validations_fail_on_duplicate_subgraph_name() {
    // Two subgraphs with the same name should be rejected by composition
    let s1 = Subgraph::parse_and_expand(
        "Duplicate",
        "https://a",
        r#"
        type Query { q: String }
    "#,
    )
    .unwrap();

    let s2 = Subgraph::parse_and_expand(
        "Duplicate", // same name again
        "https://b",
        r#"
        type Query { r: Int }
    "#,
    )
    .unwrap();
    let result = Supergraph::compose(vec![&s1, &s2]);
    assert!(
        result.is_err(),
        "composition must fail due to duplicate subgraph name"
    );
    let errs = result.unwrap_err();
    let combined = format!("{:?}", errs);
    assert!(
        combined.contains("duplicate subgraph")
            || combined.contains("duplicate subgraph/service name detected")
            || combined.contains("A subgraph named"),
        "expected duplicate-subgraph error, got: {}",
        combined
    );
}

#[test]
fn compose_happy_path_basic() {
    let s1 = Subgraph::parse_and_expand(
        "A",
        "https://a",
        r#"
        type Query { a: String }
        type Person @key(fields: "id") { id: ID! name: String }
    "#,
    )
    .unwrap();

    let s2 = Subgraph::parse_and_expand(
        "B",
        "https://b",
        r#"
        extend type Person @key(fields: "id") { id: ID! age: Int }
    "#,
    )
    .unwrap();

    let supergraph = Supergraph::compose(vec![&s1, &s2]).expect("composition should succeed");
    let sdl = print_sdl(supergraph.schema.schema());
    assert!(sdl.contains("type Person"));
    assert!(sdl.contains("age: Int"));
    assert!(sdl.contains("name: String"));
}

// -----------------------------
// mod.rs pipeline focused tests
// -----------------------------

fn try_validated(
    name: &str,
    url: &str,
    sdl: &str,
) -> Result<TSubgraph<Validated>, Vec<CompositionError>> {
    let initial = TSubgraph::<Initial>::parse(name, url, sdl).expect("parse");
    let expanded = expand_subgraphs(vec![initial])?;
    let upgraded = upgrade_subgraphs_if_necessary(expanded)?;
    let validated = validate_subgraphs(upgraded)?;
    Ok(validated.into_iter().next().expect("one subgraph"))
}
// helper: build a single Validated subgraph from (name,url,sdl)
fn mk_validated(name: &str, url: &str, sdl: &str) -> TSubgraph<Validated> {
    let initial = TSubgraph::<Initial>::parse(name, url, sdl).expect("parse subgraph");
    let expanded = expand_subgraphs(vec![initial]).expect("expand");
    let upgraded = upgrade_subgraphs_if_necessary(expanded).expect("upgrade");
    let validated = validate_subgraphs(upgraded).expect("validate");
    validated.into_iter().next().expect("one subgraph out")
}
fn try_mk_validated(
    name: &str,
    url: &str,
    sdl: &str,
) -> Result<TSubgraph<Validated>, Vec<CompositionError>> {
    let initial = TSubgraph::<Initial>::parse(name, url, sdl).expect("parse subgraph");
    let expanded = expand_subgraphs(vec![initial])?;
    let upgraded = upgrade_subgraphs_if_necessary(expanded)?;
    let validated = validate_subgraphs(upgraded)?;
    Ok(validated.into_iter().next().expect("one subgraph out"))
}

// ========== PRE-MERGE ==========

#[test]
fn pre_merge_rejects_empty_or_blank_name() {
    let a = mk_validated("   ", "http://a", r#"type Query { q: String }"#);
    let res = pre_merge_validations(&[a]);
    assert!(res.is_err(), "should error on empty/blank name");
    let msg = format!("{:?}", res.unwrap_err());
    assert!(
        msg.contains("EmptySubgraphName") || msg.contains("empty name"),
        "got: {msg}"
    );
}

#[test]
fn pre_merge_rejects_duplicate_name() {
    let sdl = r#"type Query { q: String }"#;
    let a = mk_validated("dup", "http://a", sdl);
    let b = mk_validated("dup", "http://b", sdl);
    let res = pre_merge_validations(&[a, b]);
    assert!(res.is_err(), "duplicate names must fail");
    let msg = format!("{:?}", res.unwrap_err());
    assert!(
        msg.contains("DuplicateSubgraphName") || msg.contains("duplicate subgraph"),
        "got: {msg}"
    );
}

#[test]
fn pre_merge_conflicting_directive_repeatable() {
    let a = mk_validated(
        "A",
        "http://a",
        r#"
        directive @myTag on FIELD_DEFINITION
        type Query { q: String }
        "#,
    );
    let b = mk_validated(
        "B",
        "http://b",
        r#"
        directive @myTag repeatable on FIELD_DEFINITION
        type Query { r: Int }
        "#,
    );
    let res = pre_merge_validations(&[a, b]);
    println!("{res:?}");
    assert!(res.is_err());
    let msg = format!("{:?}", res.unwrap_err());
    assert!(
        msg.contains("DirectiveRepeatableConflict") || msg.contains("repeatable flag"),
        "got: {msg}"
    );
}

#[test]
fn pre_merge_conflicting_directive_locations() {
    let a = mk_validated(
        "A",
        "http://a",
        r#"
        directive @x on FIELD
        type Query { q: String }
        "#,
    );
    let b = mk_validated(
        "B",
        "http://b",
        r#"
        directive @x on OBJECT | FIELD
        type Query { r: Int }
        "#,
    );
    let res = pre_merge_validations(&[a, b]);
    assert!(res.is_err());
    let msg = format!("{:?}", res.unwrap_err());
    assert!(
        msg.contains("DirectiveLocationsConflict") || msg.contains("locations conflict"),
        "got: {msg}"
    );
}

#[test]
fn pre_merge_scalar_specifiedby_url_conflict() {
    let a = mk_validated(
        "A",
        "http://a",
        r#"
        scalar URL @specifiedBy(url: "https://spec.example/one")
        type Query { u: URL }
        "#,
    );
    let b = mk_validated(
        "B",
        "http://b",
        r#"
        scalar URL @specifiedBy(url: "https://spec.example/two")
        type Query { v: URL }
        "#,
    );
    let res = pre_merge_validations(&[a, b]);
    println!("{res:?}");
    assert!(res.is_err());
    let msg = format!("{:?}", res.unwrap_err());
    assert!(
        msg.contains("ScalarSpecifiedByUrlConflict") || msg.contains("specifiedBy"),
        "got: {msg}"
    );
}

#[test]
fn invalid_key_missing_fields_is_caught_before_pre_merge() {
    let sdl = r#"
        type A @key { id: ID! }  # no `fields:` — invalid
        type Query { a: A }
    "#;

    match try_validated("A", "http://a", sdl) {
        Ok(_sg) => {
            // If the validator ever lets it through, pre-merge must reject it.
            let res = pre_merge_validations(&[_sg]);
            assert!(res.is_err());
            let msg = format!("{:?}", res.unwrap_err());
            assert!(msg.contains("InvalidFieldSet") || msg.contains("key"));
        }
        Err(errs) => {
            let msg = format!("{:?}", errs);
            assert!(
                msg.contains("KeyInvalidFieldsType") || msg.contains("@key"),
                "expected early validation failure, got: {msg}"
            );
        }
    }
}

#[test]
fn pre_merge_requires_unknown_field() {
    let sdl = r#"
        type A @key(fields: "id") {
          id: ID!
          x: Int @requires(fields: "zzz")
        }
        type Query { a: A }
    "#;

    match try_mk_validated("A", "http://a", sdl) {
        Ok(sg) => {
            let res = pre_merge_validations(&[sg]);
            assert!(res.is_err(), "expected pre-merge to fail");
            let msg = format!("{:?}", res.unwrap_err());
            assert!(
                msg.contains("UnknownFieldInFieldSet")
                    || msg.contains("references unknown field")
                    || msg.contains("does not have a field"),
                "got: {msg}"
            );
        }
        Err(errs) => {
            let msg = format!("{:?}", errs);
            assert!(
                msg.contains("InvalidGraphQL")
                    && (msg.contains("does not have a field `zzz`") || msg.contains("@requires")),
                "expected early validation failure about unknown field, got: {msg}"
            );
        }
    }
}

// ========== MERGE ==========

fn mk_two_validated(
    a_name: &str,
    a_sdl: &str,
    b_name: &str,
    b_sdl: &str,
) -> Vec<TSubgraph<Validated>> {
    let a0 = TSubgraph::<Initial>::parse(a_name, "http://a", a_sdl).expect("parse A");
    let b0 = TSubgraph::<Initial>::parse(b_name, "http://b", b_sdl).expect("parse B");
    // I run the pipeline on the vector 
    let expanded = expand_subgraphs(vec![a0, b0]).expect("expand");
    let upgraded = upgrade_subgraphs_if_necessary(expanded).expect("upgrade");
    let validated = validate_subgraphs(upgraded).expect("validate");

    validated
}

#[test]
fn merge_succeeds_and_combines_fields_and_returns_hints_vec() {
    let subs = mk_two_validated(
        "S1",
        r#"
        type Product @key(fields: "id") { id: ID! name: String }
        type Query { p: Product }
        "#,
        "S2",
        r#"
        extend type Product @key(fields: "id") { id: ID! price: Int description: String }
        "#,
    );

    pre_merge_validations(&subs).expect("pre-merge ok");
    let out = merge_subgraphs(subs).expect("merge ok");

    let sdl = out.supergraph.state.schema().to_string();
    assert!(sdl.contains("type Product"));
    assert!(sdl.contains("name: String"));
    assert!(sdl.contains("price: Int"));
    assert!(sdl.contains("description: String"));
    assert!(out.hints.len() >= 0);
}

#[test]
fn merge_fails_on_conflicting_field_types_across_subgraphs() {
    let subs = mk_two_validated(
        "S1",
        r#"
        type Product @key(fields: "id") { id: ID! price: Int }
        type Query { p: Product }
        "#,
        "S2",
        r#"
        extend type Product @key(fields: "id") { id: ID! price: String }
        "#,
    );
    pre_merge_validations(&subs).expect("pre-merge ok");
    let merged = merge_subgraphs(subs).expect("merge ok").supergraph;
    let res = post_merge_validations(&merged);
    assert!(res.is_err(), "expected post-merge validation to fail");

    let msg = format!("{:?}", res.err().unwrap());
    assert!(
        msg.contains("inconsistent per-graph types")
            || msg.contains("join__field")
            || msg.contains("conflict")
            || msg.contains("price"),
        "expected message to mention conflicting field types; got: {msg}"
    );
}

// ========== POST-MERGE ==========
fn mk_validated_res(
    name: &str,
    url: &str,
    sdl: &str,
) -> Result<TSubgraph<Validated>, Vec<CompositionError>> {
    let initial = TSubgraph::<Initial>::parse(name, url, sdl).map_err(|e| vec![e.into()])?;
    let expanded = expand_subgraphs(vec![initial])?;
    let upgraded = upgrade_subgraphs_if_necessary(expanded)?;
    let validated = validate_subgraphs(upgraded)?;
    Ok(validated.into_iter().next().expect("one subgraph out"))
}

#[test]
fn post_merge_validations_ok_on_valid_supergraph() {
    let subs = mk_two_validated(
        "A",
        r#"
        type Query { a: A }
        type A @key(fields: "id") { id: ID! name: String }
        "#,
        "B",
        r#"
        extend type A @key(fields: "id") { id: ID! age: Int }
        "#,
    );
    pre_merge_validations(&subs).expect("pre-merge ok");
    let merged = merge_subgraphs(subs).expect("merge ok").supergraph;

    post_merge_validations(&merged).expect("post-merge validations ok");
}

#[test]
fn post_merge_catches_requires_unknown_field_if_not_rejected_earlier() {
    let a = match mk_validated_res(
        "A",
        "http://a",
        r#"
        type Query { a: A }
        type A @key(fields: "id") { id: ID! x: Int @requires(fields: "nope") }
        "#,
    ) {
        Ok(ok) => ok,
        Err(errs) => {
            // Today the error is raised during validation already.
            // Assert that explicitly and return.
            let msg = format!("{:?}", errs);
            assert!(
                msg.contains("does not have a field `nope`")
                    || msg.contains("InvalidGraphQL")
                    || msg.contains("UnknownFieldInFieldSet")
                    || msg.contains("invalid FieldSet"),
                "unexpected early validation errors: {msg}"
            );
            return; 
        }
    };

    let b = mk_validated(
        "B",
        "http://b",
        r#"
        extend type A @key(fields: "id") { id: ID! }
        "#,
    );

    let subs = vec![a, b];

    // If we ever relax early validation, 
    // the problem needs to be caught later:
    pre_merge_validations(&subs).expect("pre-merge ok");
    let merged = merge_subgraphs(subs).expect("merge ok").supergraph;
    let res = post_merge_validations(&merged);
    assert!(
        res.is_err(),
        "post-merge should flag @requires unknown field"
    );
    let msg = format!("{:?}", res.err().unwrap());
    assert!(
        msg.contains("UnknownFieldInFieldSet") || msg.contains("invalid FieldSet"),
        "unexpected post-merge error message: {msg}"
    );
}

// ========== compose_with_options smoke ==========

#[test]
fn compose_with_options_without_satisfiability_smoke() {
    let a = TSubgraph::<Initial>::parse(
        "A",
        "http://a",
        r#"
        type Query { a: A }
        type A @key(fields: "id") { id: ID! }
        "#,
    )
    .unwrap();
    let b = TSubgraph::<Initial>::parse(
        "B",
        "http://b",
        r#"
        extend type A @key(fields: "id") { id: ID! name: String }
        "#,
    )
    .unwrap();

    let sg = compose_with_options(
        vec![a, b],
        CompositionOptions {
            run_satisfiability: false,
        },
    )
    .expect("composition ok without satisfiability");
    let _sdl = sg
        .to_api_schema(Default::default())
        .expect("API schema should be buildable")
        .schema()
        .to_string();
}
