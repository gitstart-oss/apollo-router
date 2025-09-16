mod satisfiability;

use std::vec;

pub use crate::composition::satisfiability::validate_satisfiability;
use crate::error::CompositionError;
pub use crate::schema::schema_upgrader::upgrade_subgraphs_if_necessary;
use crate::subgraph::typestate::Expanded;
use crate::subgraph::typestate::Initial;
use crate::subgraph::typestate::Subgraph;
use crate::subgraph::typestate::Upgraded;
use crate::subgraph::typestate::Validated;
pub use crate::supergraph::Merged;
pub use crate::supergraph::Satisfiable;
pub use crate::supergraph::Supergraph;

pub fn compose(
    subgraphs: Vec<Subgraph<Initial>>,
) -> Result<Supergraph<Satisfiable>, Vec<CompositionError>> {
    let expanded_subgraphs = expand_subgraphs(subgraphs)?;
    let upgraded_subgraphs = upgrade_subgraphs_if_necessary(expanded_subgraphs)?;
    let validated_subgraphs = validate_subgraphs(upgraded_subgraphs)?;

    pre_merge_validations(&validated_subgraphs)?;
    let supergraph = merge_subgraphs(validated_subgraphs)?;
    post_merge_validations(&supergraph)?;
    validate_satisfiability(supergraph)
}

/// Apollo Federation allow subgraphs to specify partial schemas (i.e. "import" directives through
/// `@link`). This function will update subgraph schemas with all missing federation definitions.
pub fn expand_subgraphs(
    subgraphs: Vec<Subgraph<Initial>>,
) -> Result<Vec<Subgraph<Expanded>>, Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = vec![];
    let expanded: Vec<Subgraph<Expanded>> = subgraphs
        .into_iter()
        .map(|s| s.expand_links())
        .filter_map(|r| r.map_err(|e| errors.push(e.into())).ok())
        .collect();
    if errors.is_empty() {
        Ok(expanded)
    } else {
        Err(errors)
    }
}

/// Validate subgraph schemas to ensure they satisfy Apollo Federation requirements (e.g. whether
/// `@key` specifies valid `FieldSet`s etc).
pub fn validate_subgraphs(
    subgraphs: Vec<Subgraph<Upgraded>>,
) -> Result<Vec<Subgraph<Validated>>, Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = vec![];
    let validated: Vec<Subgraph<Validated>> = subgraphs
        .into_iter()
        .map(|s| s.validate())
        .filter_map(|r| r.map_err(|e| errors.push(e.into())).ok())
        .collect();
    if errors.is_empty() {
        Ok(validated)
    } else {
        Err(errors)
    }
}

/// Perform validations that require information about all available subgraphs.
pub fn pre_merge_validations(
    validated_subgraphs: &[Subgraph<Validated>],
) -> Result<(), Vec<CompositionError>> {
    // Pre-merge validations - check for basic composition requirements
    // this includes checking for any conflict between subgraphs that would prevent successful merging
    if validated_subgraphs.is_empty() {
        return Err(vec![CompositionError::InvalidGraphQL {
            message: "Cannot compose an empty list of subgraphs".to_string(),
        }]);
    }

    // Check for duplicate subgraph names
    let mut seen_names = std::collections::HashSet::new();
    for subgraph in validated_subgraphs {
        if !seen_names.insert(&subgraph.name) {
            return Err(vec![CompositionError::InvalidGraphQL {
                message: format!("Duplicate subgraph name: {}", subgraph.name),
            }]);
        }
    }
    Ok(())
}

fn merge_subgraphs(
    validated_subgraphs: Vec<Subgraph<Validated>>,
) -> Result<Supergraph<Merged>, Vec<CompositionError>> {
    use crate::merger::merge::CompositionOptions;

    // Use the existing merger implementation
    let options = CompositionOptions::default();
    let merge_result = crate::merger::merge::merge_subgraphs(validated_subgraphs, options)
        .map_err(|e| {
            vec![CompositionError::InternalError {
                message: format!("Merge failed: {}", e),
            }]
        })?;

    if !merge_result.errors.is_empty() {
        return Err(merge_result.errors);
    }

    match merge_result.supergraph {
        Some(supergraph) => {
            // Convert the Valid<FederationSchema> to Supergraph<Merged>
            // This is a type state transition that indicates successful merging
            let schema = supergraph.into_inner().into_inner();
            Ok(Supergraph::<Merged>::new(
                apollo_compiler::validation::Valid::assume_valid(schema),
            ))
        }
        None => Err(vec![CompositionError::InternalError {
            message: "Merge completed but no supergraph was produced".to_string(),
        }]),
    }
}

fn post_merge_validations(supergraph: &Supergraph<Merged>) -> Result<(), Vec<CompositionError>> {
    // Post-merge validations - validate the merged supergraph
    // Based on Node.js implementation, this includes checking the final schema below
    let schema = supergraph.schema();

    // Validate that we have a query root type (required by GraphQL spec)
    if schema.schema_definition.query.is_none() {
        return Err(vec![CompositionError::InvalidGraphQL {
            message: "A valid schema must have a query root type".to_string(),
        }]);
    }
    Ok(())
}
