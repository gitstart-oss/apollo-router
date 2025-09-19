mod satisfiability;
use apollo_compiler::Name;
use apollo_compiler::ast::Value;
use apollo_compiler::schema::ExtendedType;

use crate::ValidFederationSchema;
use crate::ValidFederationSubgraph;
use crate::ValidFederationSubgraphs;
pub use crate::composition::satisfiability::validate_satisfiability;
use crate::error::CompositionError;
pub use crate::merge::MergeFailure;
pub use crate::merge::merge_federation_subgraphs;
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
use std::collections::HashMap;
use std::collections::HashSet;

pub struct CompositionOptions {
    pub run_satisfiability: bool,
}
pub struct MergeOutput {
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
    let MergeOutput {
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
    subgraphs: &[Subgraph<Validated>],
) -> Result<(), Vec<CompositionError>> {
    let mut errors = ErrorCollector::new();

    // 1. Validate subgraph names
    validate_subgraph_names(subgraphs, &mut errors);

    // 2. Validate type consistency across subgraphs
    validate_type_consistency(subgraphs, &mut errors);

    // 3. Validate directive consistency
    validate_directive_consistency(subgraphs, &mut errors);

    // 4. Validate federation directives (keys, requires, provides)
    validate_federation_directives(subgraphs, &mut errors);

    errors.finish()
}

pub fn merge_subgraphs(
    validated_subgraphs: Vec<Subgraph<Validated>>,
) -> Result<MergeOutput, Vec<CompositionError>> {
    // Convert to ValidFederationSubgraphs format
    let federation_subgraphs = convert_to_federation_subgraphs(validated_subgraphs)?;

    // Use existing merger
    match merge_federation_subgraphs(federation_subgraphs) {
        Ok(merge_success) => {
            // Convert to our format
            let supergraph = create_merged_supergraph(merge_success.schema)?;
            Ok(MergeOutput {
                supergraph,
                hints: convert_merge_warnings_to_hints(merge_success.composition_hints),
            })
        }
        Err(merge_failure) => Err(convert_merge_failure_to_composition_errors(merge_failure)),
    }
}

pub fn post_merge_validations(
    supergraph: &Supergraph<Merged>,
) -> Result<(), Vec<CompositionError>> {
    let mut errors = ErrorCollector::new();

    // 1. Validate supergraph schema structure
    validate_supergraph_structure(supergraph, &mut errors);

    // 2. Validate join directives
    validate_join_directives(supergraph, &mut errors);

    // 3. Validate entity consistency
    validate_entity_consistency(supergraph, &mut errors);

    errors.finish()
}

// ===== Validation Helper Functions =====

fn validate_subgraph_names(subgraphs: &[Subgraph<Validated>], errors: &mut ErrorCollector) {
    let mut seen_names = HashSet::new();

    for subgraph in subgraphs {
        let name = subgraph.name.trim();

        if name.is_empty() {
            errors.add_error(CompositionError::EmptySubgraphName);
            continue;
        }

        if !seen_names.insert(name.to_string()) {
            errors.add_error(CompositionError::DuplicateSubgraphName {
                name: name.to_string(),
            });
        }
    }
}

fn validate_type_consistency(subgraphs: &[Subgraph<Validated>], errors: &mut ErrorCollector) {
    let mut type_definitions: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for subgraph in subgraphs {
        let schema = subgraph.validated_schema().schema();

        for (type_name, extended_type) in &schema.types {
            let type_kind = match extended_type {
                ExtendedType::Object(_) => "Object",
                ExtendedType::Interface(_) => "Interface",
                ExtendedType::Union(_) => "Union",
                ExtendedType::Scalar(_) => "Scalar",
                ExtendedType::Enum(_) => "Enum",
                ExtendedType::InputObject(_) => "InputObject",
            };

            type_definitions
                .entry(type_name.to_string())
                .or_default()
                .push((subgraph.name.clone(), type_kind.to_string()));
        }
    }

    // Check for type kind conflicts
    for (type_name, definitions) in type_definitions {
        let kinds: HashSet<String> = definitions.iter().map(|(_, kind)| kind.clone()).collect();
        if kinds.len() > 1 {
            let conflicts = definitions
                .iter()
                .map(|(subgraph, kind)| format!("{}: {}", subgraph, kind))
                .collect::<Vec<_>>()
                .join(", ");

            errors.add_error(CompositionError::ConflictingTypeDefinitions {
                type_name,
                conflicts,
            });
        }
    }
}

fn validate_directive_consistency(subgraphs: &[Subgraph<Validated>], errors: &mut ErrorCollector) {
    let mut directive_definitions: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for subgraph in subgraphs {
        let schema = subgraph.validated_schema().schema();

        for (directive_name, directive_def) in &schema.directive_definitions {
            let directive_signature = format!("{:?}", directive_def);
            directive_definitions
                .entry(directive_name.to_string())
                .or_default()
                .push((subgraph.name.clone(), directive_signature));
        }
    }

    for (directive_name, definitions) in directive_definitions {
        let signatures: HashSet<String> = definitions.iter().map(|(_, sig)| sig.clone()).collect();
        if signatures.len() > 1 {
            let conflicts = definitions
                .iter()
                .map(|(subgraph, sig)| format!("{}: {}", subgraph, sig))
                .collect::<Vec<_>>()
                .join(", ");

            errors.add_error(CompositionError::ConflictingDirectiveDefinitions {
                directive_name,
                details: conflicts,
            });
        }
    }
}

fn validate_federation_directives(subgraphs: &[Subgraph<Validated>], errors: &mut ErrorCollector) {
    for subgraph in subgraphs {
        let schema = subgraph.validated_schema().schema();

        for (type_name, extended_type) in &schema.types {
            if let ExtendedType::Object(object_type) = extended_type {
                for directive in &object_type.directives {
                    match directive.name.as_str() {
                        "key" => validate_key_directive(
                            type_name,
                            directive,
                            schema,
                            &subgraph.name,
                            errors,
                        ),
                        _ => {}
                    }
                }

                for (field_name, field_def) in &object_type.fields {
                    for directive in &field_def.directives {
                        match directive.name.as_str() {
                            "requires" | "provides" => validate_field_directive(
                                type_name,
                                field_name,
                                directive,
                                schema,
                                &subgraph.name,
                                errors,
                            ),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

fn validate_key_directive(
    type_name: &Name,
    directive: &apollo_compiler::ast::Directive,
    schema: &apollo_compiler::Schema,
    subgraph_name: &str,
    errors: &mut ErrorCollector,
) {
    if let Some(arg) = directive
        .arguments
        .iter()
        .find(|arg| arg.name.as_str() == "fields")
    {
        if let Value::String(field_set) = &*arg.value {
            validate_field_set_exists(type_name, field_set, schema, "key", subgraph_name, errors);
        }
    }
}

fn validate_field_directive(
    type_name: &Name,
    _field_name: &Name,
    directive: &apollo_compiler::ast::Directive,
    schema: &apollo_compiler::Schema,
    subgraph_name: &str,
    errors: &mut ErrorCollector,
) {
    if let Some(arg) = directive
        .arguments
        .iter()
        .find(|arg| arg.name.as_str() == "fields")
    {
        if let Value::String(field_set) = &*arg.value {
            validate_field_set_exists(
                type_name,
                field_set,
                schema,
                directive.name.as_str(),
                subgraph_name,
                errors,
            );
        }
    }
}

fn validate_field_set_exists(
    type_name: &Name,
    field_set: &str,
    schema: &apollo_compiler::Schema,
    directive_name: &str,
    _subgraph_name: &str,
    errors: &mut ErrorCollector,
) {
    if let Some(ExtendedType::Object(object_type)) = schema.types.get(type_name) {
        let field_names: Vec<&str> = field_set.split_whitespace().collect();
        for field_name in field_names {
            let clean_field = field_name.trim();
            if let Ok(parsed_field) = Name::try_from(clean_field) {
                if !object_type.fields.contains_key(&parsed_field) {
                    errors.add_error(CompositionError::FieldSetReferencesNonexistentField {
                        directive: directive_name.to_string(),
                        type_name: type_name.to_string(),
                        field: clean_field.to_string(),
                    });
                }
            }
        }
    }
}

fn validate_supergraph_structure(supergraph: &Supergraph<Merged>, errors: &mut ErrorCollector) {
    let schema = supergraph.state.schema();

    if !schema.types.contains_key(&Name::try_from("Query").unwrap()) {
        errors.add_error(CompositionError::InternalError {
            message: "Supergraph must have a Query type".to_string(),
        });
    }

    for (type_name, extended_type) in &schema.types {
        match extended_type {
            ExtendedType::Object(object_type) => {
                if object_type.fields.is_empty() {
                    errors.add_error(CompositionError::TypeDefinitionInvalid {
                        message: format!("Object type '{}' has no fields", type_name),
                    });
                }
            }
            ExtendedType::Interface(interface_type) => {
                if interface_type.fields.is_empty() {
                    errors.add_error(CompositionError::TypeDefinitionInvalid {
                        message: format!("Interface type '{}' has no fields", type_name),
                    });
                }
            }
            ExtendedType::Union(union_type) => {
                if union_type.members.is_empty() {
                    errors.add_error(CompositionError::TypeDefinitionInvalid {
                        message: format!("Union type '{}' has no members", type_name),
                    });
                }
            }
            _ => {}
        }
    }
}

fn validate_join_directives(supergraph: &Supergraph<Merged>, errors: &mut ErrorCollector) {
    let schema = supergraph.state.schema();

    for (type_name, extended_type) in &schema.types {
        if let ExtendedType::Object(object_type) = extended_type {
            for directive in &object_type.directives {
                if directive.name.as_str().starts_with("join__") {
                    validate_join_directive_consistency(type_name, directive, schema, errors);
                }
            }

            for (field_name, field_def) in &object_type.fields {
                for directive in &field_def.directives {
                    if directive.name.as_str().starts_with("join__") {
                        validate_join_field_directive(
                            type_name, field_name, directive, schema, errors,
                        );
                    }
                }
            }
        }
    }
}

fn validate_join_directive_consistency(
    type_name: &Name,
    directive: &apollo_compiler::ast::Directive,
    _schema: &apollo_compiler::Schema,
    errors: &mut ErrorCollector,
) {
    // Basic validation for join directives
    match directive.name.as_str() {
        "join__type" => {
            if let Some(arg) = directive
                .arguments
                .iter()
                .find(|arg| arg.name.as_str() == "graph")
            {
                if let Value::String(graph_name) = &*arg.value {
                    if graph_name.is_empty() {
                        errors.add_error(CompositionError::TypeDefinitionInvalid {
                            message: format!(
                                "join__type directive on '{}' has empty graph name",
                                type_name
                            ),
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

fn validate_join_field_directive(
    type_name: &Name,
    field_name: &Name,
    directive: &apollo_compiler::ast::Directive,
    _schema: &apollo_compiler::Schema,
    errors: &mut ErrorCollector,
) {
    match directive.name.as_str() {
        "join__field" => {
            if let Some(arg) = directive
                .arguments
                .iter()
                .find(|arg| arg.name.as_str() == "graph")
            {
                if let Value::String(graph_name) = &*arg.value {
                    if graph_name.is_empty() {
                        errors.add_error(CompositionError::TypeDefinitionInvalid {
                            message: format!(
                                "join__field directive on '{}.{}' has empty graph name",
                                type_name, field_name
                            ),
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

fn validate_entity_consistency(supergraph: &Supergraph<Merged>, errors: &mut ErrorCollector) {
    let schema = supergraph.state.schema();
    let mut entities: HashMap<String, Vec<String>> = HashMap::new();

    for (type_name, extended_type) in &schema.types {
        if let ExtendedType::Object(object_type) = extended_type {
            let mut entity_keys = Vec::new();

            for directive in &object_type.directives {
                if directive.name.as_str() == "key" {
                    if let Some(arg) = directive
                        .arguments
                        .iter()
                        .find(|arg| arg.name.as_str() == "fields")
                    {
                        if let Value::String(field_set) = &*arg.value {
                            entity_keys.push(field_set.clone());
                        }
                    }
                }
            }

            if !entity_keys.is_empty() {
                entities.insert(type_name.to_string(), entity_keys);
            }
        }
    }

    for (type_name, keys) in entities {
        if keys.is_empty() {
            errors.add_error(CompositionError::TypeDefinitionInvalid {
                message: format!("Entity '{}' has no valid @key directives", type_name),
            });
        }

        for key in &keys {
            if key.trim().is_empty() {
                errors.add_error(CompositionError::InvalidFieldSet {
                    directive: "key".to_string(),
                    location: type_name.clone(),
                    field_set: key.clone(),
                    reason: "Field set cannot be empty".to_string(),
                });
            }
        }
    }
}

// ===== Error Collection and Conversion Utilities =====

/// Utility for collecting validation errors in a builder pattern
pub struct ErrorCollector {
    errors: Vec<CompositionError>,
}

impl ErrorCollector {
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    pub fn add_error(&mut self, error: CompositionError) {
        self.errors.push(error);
    }

    pub fn add_errors(&mut self, mut errors: Vec<CompositionError>) {
        self.errors.append(&mut errors);
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn finish(self) -> Result<(), Vec<CompositionError>> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }

    pub fn finish_with<T>(self, value: T) -> Result<T, Vec<CompositionError>> {
        if self.errors.is_empty() {
            Ok(value)
        } else {
            Err(self.errors)
        }
    }
}

/// Convert MergeFailure to CompositionError list
fn convert_merge_failure_to_composition_errors(failure: MergeFailure) -> Vec<CompositionError> {
    failure
        .errors
        .into_iter()
        .map(|e| CompositionError::TypeDefinitionInvalid { message: e })
        .collect()
}

/// Convert MergeWarning list to hint strings  
fn convert_merge_warnings_to_hints(warnings: Vec<String>) -> Vec<String> {
    warnings
}

/// Convert validated subgraphs to ValidFederationSubgraphs format
fn convert_to_federation_subgraphs(
    validated_subgraphs: Vec<Subgraph<Validated>>,
) -> Result<ValidFederationSubgraphs, Vec<CompositionError>> {
    let mut federation_subgraphs = ValidFederationSubgraphs::new();
    let mut errors = ErrorCollector::new();

    for subgraph in validated_subgraphs {
        let federation_subgraph = ValidFederationSubgraph {
            name: subgraph.name.clone(),
            url: subgraph.url.clone(),
            schema: subgraph.validated_schema().clone(),
        };

        if let Err(_) = federation_subgraphs.add(federation_subgraph) {
            errors.add_error(CompositionError::DuplicateSubgraphName {
                name: subgraph.name,
            });
        }
    }

    errors.finish_with(federation_subgraphs)
}

/// Create a Supergraph<Merged> from a valid schema
fn create_merged_supergraph(
    schema: apollo_compiler::validation::Valid<apollo_compiler::Schema>,
) -> Result<Supergraph<Merged>, Vec<CompositionError>> {
    Ok(Supergraph::<Merged>::new(schema))
}
