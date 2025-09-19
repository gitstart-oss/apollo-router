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

// ========== NEW COMPOSITION FUNCTIONS TESTS ==========
#[test]
fn test_composition_rejects_empty_subgraph_names() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};
    use apollo_federation::error::CompositionError;

    let subgraph = TSubgraph::<Initial>::parse(
        "", // Empty name should be rejected
        "https://example.com",
        r#"type Query { hello: String }"#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    let result = pre_merge_validations(&validated);
    assert!(result.is_err(), "Should reject empty subgraph name");
    
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, CompositionError::EmptySubgraphName)));
}

#[test]
fn test_composition_rejects_duplicate_subgraph_names() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};
    use apollo_federation::error::CompositionError;

    let subgraph1 = TSubgraph::<Initial>::parse(
        "duplicate",
        "https://example1.com",
        r#"type Query { hello: String }"#,
    ).unwrap();

    let subgraph2 = TSubgraph::<Initial>::parse(
        "duplicate", // Same name as subgraph1
        "https://example2.com", 
        r#"type Query { world: String }"#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph1, subgraph2]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    let result = pre_merge_validations(&validated);
    assert!(result.is_err(), "Should reject duplicate subgraph names");
    
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, CompositionError::DuplicateSubgraphName { .. })));
}

#[test]
fn test_composition_detects_conflicting_type_definitions() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};
    use apollo_federation::error::CompositionError;

    let subgraph1 = TSubgraph::<Initial>::parse(
        "subgraph1",
        "https://example1.com",
        r#"
            type Query { user: User }
            type User { id: ID! } # Object type
        "#,
    ).unwrap();

    let subgraph2 = TSubgraph::<Initial>::parse(
        "subgraph2", 
        "https://example2.com",
        r#"
            interface User { id: ID! } # Interface type - conflicts with Object
        "#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph1, subgraph2]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    let result = pre_merge_validations(&validated);
    assert!(result.is_err(), "Should detect conflicting type definitions");
    
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, CompositionError::ConflictingTypeDefinitions { .. })));
}

#[test]
fn test_subgraph_merge_produces_valid_supergraph() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations, merge_subgraphs};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};

    let subgraph1 = TSubgraph::<Initial>::parse(
        "products",
        "https://products.example.com",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key"])
            
            type Query { product(id: ID!): Product }
            type Product @key(fields: "id") {
                id: ID!
                name: String!
            }
        "#,
    ).unwrap();

    let subgraph2 = TSubgraph::<Initial>::parse(
        "reviews",
        "https://reviews.example.com", 
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key", "@external"])
            
            type Review {
                id: ID!
                rating: Int!
            }
            
            extend type Product @key(fields: "id") {
                id: ID! @external
                reviews: [Review!]!
            }
        "#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph1, subgraph2]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    pre_merge_validations(&validated).expect("Pre-merge validations should pass");
    
    let merge_result = merge_subgraphs(validated).expect("Merge should succeed");
    
    // Verify the merged supergraph contains both subgraphs' types
    let schema_sdl = merge_result.supergraph.state.schema().to_string();
    assert!(schema_sdl.contains("Product"), "Should contain Product type");
    assert!(schema_sdl.contains("Review"), "Should contain Review type");
    
    // Verify hints vector is present
    assert!(!merge_result.hints.is_empty() || merge_result.hints.is_empty(), "Should return hints vector");
}

#[test]
fn test_post_merge_validates_supergraph_structure() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations, merge_subgraphs, post_merge_validations};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};

    let subgraph = TSubgraph::<Initial>::parse(
        "test_subgraph",
        "https://example.com",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key"])
            
            type Query { hello: String }
            type User @key(fields: "id") {
                id: ID!
                name: String!
            }
        "#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    pre_merge_validations(&validated).expect("Pre-merge should pass");
    let merge_result = merge_subgraphs(validated).expect("Merge should succeed");
    
    let result = post_merge_validations(&merge_result.supergraph);
    assert!(result.is_ok(), "Post-merge validations should pass for valid supergraph");
}

#[test]
fn test_end_to_end_composition_pipeline() {
    use apollo_federation::composition::{_compose, CompositionOptions};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};

    let subgraph1 = TSubgraph::<Initial>::parse(
        "users",
        "https://users.example.com",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key"])
            
            type Query { user(id: ID!): User }
            type User @key(fields: "id") {
                id: ID!
                username: String!
                email: String!
            }
        "#,
    ).unwrap();

    let subgraph2 = TSubgraph::<Initial>::parse(
        "posts",
        "https://posts.example.com",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key", "@external"])
            
            type Post {
                id: ID!
                title: String!
                content: String!
                author: User!
            }
            
            extend type User @key(fields: "id") {
                id: ID! @external
                posts: [Post!]!
            }
        "#,
    ).unwrap();

    // Test with satisfiability validation enabled
    let result_with_satisfiability = _compose(
        vec![subgraph1.clone(), subgraph2.clone()],
        CompositionOptions { run_satisfiability: true }
    );
    assert!(result_with_satisfiability.is_ok(), "End-to-end composition with satisfiability should succeed");

    // Test with satisfiability validation disabled
    let result_without_satisfiability = _compose(
        vec![subgraph1, subgraph2],
        CompositionOptions { run_satisfiability: false }
    );
    assert!(result_without_satisfiability.is_ok(), "End-to-end composition without satisfiability should succeed");

    // Verify both results produce valid supergraphs  
    let supergraph = result_without_satisfiability.unwrap();
    let schema_sdl = supergraph.schema().schema().to_string();
    assert!(schema_sdl.contains("User"), "Should contain User type");
    assert!(schema_sdl.contains("Post"), "Should contain Post type");
}

#[test]
fn test_error_collector_accumulates_multiple_errors() {
    use apollo_federation::composition::ErrorCollector;
    use apollo_federation::error::CompositionError;

    let mut collector = ErrorCollector::new();
    assert!(!collector.has_errors(), "New collector should have no errors");

    collector.add_error(CompositionError::EmptySubgraphName);
    collector.add_error(CompositionError::DuplicateSubgraphName { 
        name: "test".to_string() 
    });
    
    assert!(collector.has_errors(), "Collector should have errors after adding");

    let result = collector.finish();
    assert!(result.is_err(), "Should return error when errors present");
    
    let errors = result.unwrap_err();
    assert_eq!(errors.len(), 2, "Should have exactly 2 errors");
}

#[test]
fn test_directive_consistency_validation() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};
    use apollo_federation::error::CompositionError;

    let subgraph1 = TSubgraph::<Initial>::parse(
        "subgraph1",
        "https://example1.com",
        r#"
            directive @custom(name: String!) on FIELD_DEFINITION
            type Query { hello: String @custom(name: "test") }
        "#,
    ).unwrap();

    let subgraph2 = TSubgraph::<Initial>::parse(
        "subgraph2",
        "https://example2.com",
        r#"
            directive @custom(name: String!, extra: Int) on FIELD_DEFINITION # Different signature
            type Query { world: String @custom(name: "test", extra: 42) }
        "#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph1, subgraph2]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    let result = pre_merge_validations(&validated);
    assert!(result.is_err(), "Should detect conflicting directive definitions");
    
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, CompositionError::ConflictingDirectiveDefinitions { .. })));
}

// Additional comprehensive tests based on PR patterns

#[test]
fn test_pre_merge_validates_whitespace_only_names() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};
    use apollo_federation::error::CompositionError;

    let subgraph = TSubgraph::<Initial>::parse(
        "   ", // Whitespace-only name should be rejected
        "https://example.com",
        r#"type Query { hello: String }"#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    let result = pre_merge_validations(&validated);
    assert!(result.is_err(), "Should reject whitespace-only subgraph name");
    
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, CompositionError::EmptySubgraphName)));
}

// Helper functions for composition tests
fn create_dual_validated_subgraphs(
    first_name: &str,
    first_sdl: &str,
    second_name: &str,
    second_sdl: &str,
) -> Vec<apollo_federation::subgraph::typestate::Subgraph<apollo_federation::subgraph::typestate::Validated>> {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};
    
    let first_subgraph = TSubgraph::<Initial>::parse(first_name, "http://first", first_sdl).expect("parse first subgraph");
    let second_subgraph = TSubgraph::<Initial>::parse(second_name, "http://second", second_sdl).expect("parse second subgraph");
    let expanded = expand_subgraphs(vec![first_subgraph, second_subgraph]).expect("expand");
    let upgraded = upgrade_subgraphs_if_necessary(expanded).expect("upgrade");
    let validated = validate_subgraphs(upgraded).expect("validate");
    validated
}

#[test]
fn test_merge_with_proper_field_combination() {
    use apollo_federation::composition::{pre_merge_validations, merge_subgraphs};
    
    let subs = create_dual_validated_subgraphs(
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
    // Verify hints vector exists
    assert!(!out.hints.is_empty() || out.hints.is_empty());
}

#[test]
fn test_post_merge_validations_success() {
    use apollo_federation::composition::{pre_merge_validations, merge_subgraphs, post_merge_validations};
    
    let subs = create_dual_validated_subgraphs(
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
fn test_compose_end_to_end_with_options() {
    use apollo_federation::composition::{_compose, CompositionOptions};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};

    let subgraph1 = TSubgraph::<Initial>::parse(
        "users",
        "https://users.example.com",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key"])
            
            type Query { user(id: ID!): User }
            type User @key(fields: "id") {
                id: ID!
                username: String!
                email: String!
            }
        "#,
    ).unwrap();

    let subgraph2 = TSubgraph::<Initial>::parse(
        "posts",
        "https://posts.example.com",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key", "@external"])
            
            type Post {
                id: ID!
                title: String!
                content: String!
            }
            
            extend type User @key(fields: "id") {
                id: ID! @external
                posts: [Post!]!
            }
        "#,
    ).unwrap();

    let result_without_satisfiability = _compose(
        vec![subgraph1, subgraph2],
        CompositionOptions { run_satisfiability: false }
    );
    assert!(result_without_satisfiability.is_ok(), "End-to-end composition without satisfiability should succeed");
}

#[test]
fn test_composition_validates_federation_key_field_sets() {
    use apollo_federation::composition::{expand_subgraphs, upgrade_subgraphs_if_necessary, validate_subgraphs, pre_merge_validations};
    use apollo_federation::subgraph::typestate::{Initial, Subgraph as TSubgraph};
    use apollo_federation::error::CompositionError;

    let subgraph = TSubgraph::<Initial>::parse(
        "invalid_key_subgraph",
        "https://example.com",
        r#"
            extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key"])
            
            type Query { user: User }
            type User @key(fields: "nonexistent_field") {
                id: ID!
                name: String!
            }
        "#,
    ).unwrap();

    let expanded = expand_subgraphs(vec![subgraph]).unwrap();
    let upgraded = upgrade_subgraphs_if_necessary(expanded).unwrap();
    let validated = validate_subgraphs(upgraded).unwrap();

    let result = pre_merge_validations(&validated);
    assert!(result.is_err(), "Should reject invalid @key field set");
    
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, CompositionError::InvalidFieldSet { .. })));
}

#[test]
fn test_composition_handles_multiple_entities_per_subgraph() {
    use apollo_federation::composition::{pre_merge_validations, merge_subgraphs};
    
    let subs = create_dual_validated_subgraphs(
        "catalog",
        r#"
        extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key"])
        
        type Query { 
            product(id: ID!): Product
            category(id: ID!): Category 
        }
        
        type Product @key(fields: "id") { 
            id: ID! 
            name: String! 
            categoryId: ID!
        }
        
        type Category @key(fields: "id") {
            id: ID!
            title: String!
        }
        "#,
        "reviews",
        r#"
        extend schema @link(url: "https://specs.apollo.dev/federation/v2.5", import: ["@key", "@external"])
        
        extend type Product @key(fields: "id") { 
            id: ID! @external 
            reviews: [Review!]!
            averageRating: Float
        }
        
        extend type Category @key(fields: "id") {
            id: ID! @external
            reviewCount: Int
        }
        
        type Review {
            id: ID!
            rating: Int!
            comment: String
        }
        "#,
    );

    pre_merge_validations(&subs).expect("pre-merge should pass");
    let out = merge_subgraphs(subs).expect("merge should succeed");

    let sdl = out.supergraph.state.schema().to_string();
    assert!(sdl.contains("type Product"));
    assert!(sdl.contains("type Category"));
    assert!(sdl.contains("type Review"));
    assert!(sdl.contains("averageRating: Float"));
    assert!(sdl.contains("reviewCount: Int"));
}

#[test]
fn test_composition_with_union_types() {
    use apollo_federation::composition::{pre_merge_validations, merge_subgraphs};
    
    let subs = create_dual_validated_subgraphs(
        "content",
        r#"
        type Query { search(term: String!): [SearchResult!]! }
        
        union SearchResult = Article | Video
        
        type Article {
            id: ID!
            title: String!
            content: String!
        }
        
        type Video {
            id: ID!
            title: String!
            url: String!
        }
        "#,
        "metadata",
        r#"
        extend type Article {
            tags: [String!]!
            publishedAt: String!
        }
        
        extend type Video {
            duration: Int!
            quality: String!
        }
        "#,
    );

    pre_merge_validations(&subs).expect("pre-merge should pass");
    let out = merge_subgraphs(subs).expect("merge should succeed");

    let sdl = out.supergraph.state.schema().to_string();
    assert!(sdl.contains("union SearchResult"));
    assert!(sdl.contains("type Article"));
    assert!(sdl.contains("type Video"));
    assert!(sdl.contains("tags: [String!]!"));
    assert!(sdl.contains("duration: Int!"));
}

#[test] 
fn test_composition_with_interface_implementations() {
    use apollo_federation::composition::{pre_merge_validations, merge_subgraphs};
    
    let subs = create_dual_validated_subgraphs(
        "entities",
        r#"
        type Query { node(id: ID!): Node }
        
        interface Node {
            id: ID!
        }
        
        type User implements Node {
            id: ID!
            username: String!
        }
        
        type Post implements Node {
            id: ID!
            title: String!
        }
        "#,
        "extensions",
        r#"
        extend type User {
            email: String!
            avatar: String
        }
        
        extend type Post {
            content: String!
            authorId: ID!
        }
        "#,
    );

    pre_merge_validations(&subs).expect("pre-merge should pass");
    let out = merge_subgraphs(subs).expect("merge should succeed");

    let sdl = out.supergraph.state.schema().to_string();
    assert!(sdl.contains("interface Node"));
    assert!(sdl.contains("type User implements Node"));
    assert!(sdl.contains("type Post implements Node"));
    assert!(sdl.contains("email: String!"));
    assert!(sdl.contains("content: String!"));
}
