mod satisfiability;

use std::vec;

use crate::ValidFederationSchema;
pub use crate::composition::satisfiability::validate_satisfiability;
use crate::error::CompositionError;
pub use crate::schema::schema_upgrader::upgrade_subgraphs_if_necessary;
use crate::subgraph::typestate::Expanded;
use crate::subgraph::typestate::Initial;
use crate::subgraph::typestate::Subgraph;
use crate::subgraph::typestate::Upgraded;
use crate::subgraph::typestate::Validated;
use crate::supergraph::CompositionHint;
pub use crate::supergraph::Merged;
pub use crate::supergraph::Satisfiable;
pub use crate::supergraph::Supergraph;

pub struct CompositionOptions {
    /// Whether to run satisfiability checks after composition. Default: `true`.
    pub run_satisfiability: bool,
}
pub struct MergeResult {
    pub supergraph: Supergraph<Merged>,
    pub hints: Vec<String>,
}

impl Default for CompositionOptions {
    fn default() -> Self {
        Self {
            run_satisfiability: true,
        }
    }
}

fn validate_options(_options: &CompositionOptions) -> Result<(), CompositionError> {
    // Since this could potentially grow in complexity, we can validate other options here.
    Ok(())
}

pub fn _compose(
    subgraphs: Vec<Subgraph<Initial>>,
    options: CompositionOptions,
) -> Result<Supergraph<Satisfiable>, Vec<CompositionError>> {
    if let Err(e) = validate_options(&options) {
        return Err(vec![e]);
    }

    let expanded_subgraphs = expand_subgraphs(subgraphs)?;
    let upgraded_subgraphs = upgrade_subgraphs_if_necessary(expanded_subgraphs)?;
    let validated_subgraphs = validate_subgraphs(upgraded_subgraphs)?;

    pre_merge_validations(&validated_subgraphs)?;
    let MergeResult {
        supergraph: merged_supergraph,
        hints: _merge_hints,
    } = merge_subgraphs(validated_subgraphs)?;
    post_merge_validations(&merged_supergraph)?;

    if options.run_satisfiability {
        let satisfiable = validate_satisfiability(merged_supergraph)?;
        Ok(satisfiable)
    } else {
        let vfs =
            ValidFederationSchema::new(merged_supergraph.state.schema().clone()).map_err(|e| {
                vec![CompositionError::InternalError {
                    message: format!("failed to construct ValidFederationSchema: {:?}", e),
                }]
            })?;
        Ok(Supergraph::<Satisfiable>::new(
            vfs,
            Vec::<CompositionHint>::new(),
        ))
    }
}

pub fn compose(
    subgraphs: Vec<Subgraph<Initial>>,
) -> Result<Supergraph<Satisfiable>, Vec<CompositionError>> {
    _compose(subgraphs, CompositionOptions::default())
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
    _subgraphs: &[Subgraph<Validated>],
) -> Result<(), Vec<CompositionError>> {
    Err(vec![CompositionError::InternalError {
        message: "pre_merge_validations is not implemented yet".to_string(),
    }])
}

pub fn merge_subgraphs(
    _subgraphs: Vec<Subgraph<Validated>>,
) -> Result<MergeResult, Vec<CompositionError>> {
    Err(vec![CompositionError::InternalError {
        message: "merge_subgraphs is not implemented yet".to_string(),
    }])
}

pub fn post_merge_validations(
    _supergraph: &Supergraph<Merged>,
) -> Result<(), Vec<CompositionError>> {
    Err(vec![CompositionError::InternalError {
        message: "post_merge_validations is not implemented yet".to_string(),
    }])
}
