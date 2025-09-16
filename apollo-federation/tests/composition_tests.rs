use apollo_compiler::Schema;
use apollo_federation::Supergraph;
use apollo_federation::subgraph::Subgraph;

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
fn can_compose_valid_subgraphs() {
    let s1 = Subgraph::parse_and_expand(
        "Users",
        "https://users",
        r#"
            type Query {
                user(id: ID!): User
            }

            type User @key(fields: "id") {
                id: ID!
                name: String!
            }
        "#,
    )
    .unwrap();

    let s2 = Subgraph::parse_and_expand(
        "Products",
        "https://products",
        r#"
            type Query {
                product(id: ID!): Product
            }

            type Product @key(fields: "id") {
                id: ID!
                title: String!
            }

            type User @key(fields: "id") {
                id: ID!
                orders: [Product!]!
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
fn compose_handles_empty_subgraphs() {
    let result = Supergraph::compose(vec![]);
    assert!(
        result.is_ok(),
        "Composition with empty subgraphs should succeed and return empty supergraph"
    );
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
fn compose_fails_on_pre_merge_validation_errors() {
    // Test that composition fails during pre-merge validation due to duplicate subgraph names
    let s1 = Subgraph::parse_and_expand(
        "SubgraphA",
        "https://subgraph1",
        r#"
            type Query {
                user(id: ID!): User
            }

            type User @key(fields: "id") {
                id: ID!
                name: String!
            }
        "#,
    )
    .unwrap();

    let s2 = Subgraph::parse_and_expand(
        "SubgraphA",
        "https://subgraph2",
        r#"
            type Query {
                profile(id: ID!): Profile
            }

            type Profile @key(fields: "id") {
                id: ID!
                email: String!
            }
        "#,
    )
    .unwrap();

    // Composition should fail due to pre-merge validation errors (duplicate names)
    let result = Supergraph::compose(vec![&s1, &s2]);

    assert!(
        result.is_err(),
        "Composition should fail when subgraphs have duplicate names"
    );

    // Use insta to snapshot the error for verification
    insta::assert_debug_snapshot!(result);
}
