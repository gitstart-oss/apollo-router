mod satisfiability;

use std::vec;

pub use crate::composition::satisfiability::validate_satisfiability;
use crate::error::CompositionError;
use crate::error::FederationError;
use crate::error::MultipleFederationErrors;
pub use crate::schema::schema_upgrader::upgrade_subgraphs_if_necessary;
use crate::subgraph::ValidSubgraph;
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
    subgraphs: &[Subgraph<Validated>],
) -> Result<(), Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = vec![];

    for subgraph in subgraphs {
        if let Err(error) = subgraph.schema().clone().into_inner().validate() {
            errors.push(CompositionError::InternalError {
                message: error.to_string(),
            })
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn merge_subgraphs(
    subgraphs: Vec<Subgraph<Validated>>,
) -> Result<Supergraph<Merged>, Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = vec![];

    let valid_subgraphs: Vec<ValidSubgraph> =
        subgraphs.iter().map(|s| s.to_valid_subgraph()).collect();

    crate::merge::merge_subgraphs(valid_subgraphs.iter().collect())
        .map(|merged| Supergraph::<Merged>::new(merged.schema))
        .map_err(|e| {
            errors.push(CompositionError::SubgraphError {
                subgraph: e.schema.unwrap().to_string(),
                error: FederationError::MultipleFederationErrors(MultipleFederationErrors {
                    errors: e
                        .errors
                        .iter()
                        .map(|e| crate::error::SingleFederationError::Internal {
                            message: e.clone(),
                        })
                        .collect(),
                }),
            });

            errors
        })
}

pub fn post_merge_validations(
    supergraph: &Supergraph<Merged>,
) -> Result<(), Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = vec![];
    supergraph
        .schema()
        .clone()
        .into_inner()
        .validate()
        .map(|_| ())
        .map_err(|error| {
            errors.push(CompositionError::InternalError {
                message: error.to_string(),
            });

            errors
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subgraph::typestate::Initial;

    // Helper: create a Subgraph<Validated> from SDL by running it through the real pipeline.
    fn make_validated_subgraph(name: &str, sdl: &str) -> Result<Subgraph<Validated>, String> {
        // Parse the SDL into a Schema and construct an initial Subgraph with a dummy URL.
        let schema = apollo_compiler::schema::Schema::parse(sdl, ".").expect("parse SDL to Schema");
        let initial: Subgraph<Initial> = Subgraph::new(name, "http://localhost", schema);
        let expanded = expand_subgraphs(vec![initial]).expect("expand links");
        let upgraded =
            upgrade_subgraphs_if_necessary(expanded).expect("upgrade subgraphs if necessary");
        let validated = validate_subgraphs(upgraded).expect("validate subgraphs");
        Ok(validated
            .into_iter()
            .next()
            .expect("one validated subgraph"))
    }

    #[test]
    fn pre_merge_validations_ok_new_first() {
        let sdl = r#"
            type Query {
              hello: String
            }
        "#;
        let validated = make_validated_subgraph("svc1", sdl).expect("invalid subgraph");
        let result = pre_merge_validations(&[validated.clone()]);

        assert!(result.is_ok());

        let merger_result = merge_subgraphs(vec![validated]);

        assert!(merger_result.is_ok());

        let post_merge_validation = post_merge_validations(&merger_result.as_ref().unwrap());

        assert!(post_merge_validation.is_ok());

        assert!(validate_satisfiability(merger_result.unwrap()).is_ok())
    }
}
