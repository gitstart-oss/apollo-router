mod satisfiability;
use crate::ValidFederationSchema;
use crate::ValidFederationSubgraph;
use crate::ValidFederationSubgraphs;
pub use crate::composition::satisfiability::validate_satisfiability;
use crate::error::CompositionError;
pub use crate::merge::merge_federation_subgraphs;
pub use crate::schema::schema_upgrader::upgrade_subgraphs_if_necessary;
use crate::subgraph::typestate::Expanded;
use crate::subgraph::typestate::Initial;
use crate::subgraph::typestate::Subgraph;
use crate::subgraph::typestate::Upgraded;
use crate::subgraph::typestate::Validated;
pub use crate::supergraph::Merged;
pub use crate::supergraph::Satisfiable;
pub use crate::supergraph::Supergraph;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::vec;

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
/// how to access a Subgraph's schema/types (see comments below).
pub fn pre_merge_validations(
    subgraphs: &[Subgraph<Validated>],
) -> Result<(), Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = Vec::new();

    // Duplicate-name / empty-name check
    let mut seen: HashSet<String> = HashSet::new();
    for sg in subgraphs.iter() {
        let name = &sg.name;
        if name.trim().is_empty() {
            errors.push(CompositionError::InternalError {
                message: "subgraph/service has an empty name".to_string(),
            });
            continue;
        }
        if !seen.insert(name.to_string()) {
            errors.push(CompositionError::InternalError {
                message: format!("duplicate subgraph/service name detected: '{}'", name),
            });
        }
    }
    // Optional: simple sanity check that each subgraph has at least one definition.
    // for sg in subgraphs.iter() {
    //     let type_count = sg.type_count();
    //     if type_count == 0 {
    //         errors.push(CompositionError::InternalError {
    //             message: format!("subgraph '{}' has no type definitions", &sg.name),
    //         });
    //     }
    // }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(vec![CompositionError::InternalError {
            message: "pre_merge_validations is not implemented yet".to_string(),
        }])
    }
}
pub struct MergeResult {
    pub supergraph: Supergraph<Merged>,
    pub hints: Vec<String>, // optional, port from Node `hints`
}
fn to_valid_federation_subgraph(
    sg: Subgraph<Validated>,
) -> Result<ValidFederationSubgraph, CompositionError> {
    let validated_schema = sg.validated_schema().clone();
    let name_string = sg.name;
    let url_string = sg.url;
    let vfs = ValidFederationSubgraph {
        name: name_string.clone(),
        url: url_string,
        schema: validated_schema,
    };

    Ok(vfs)
}
fn vec_to_valid_federation_subgraphs(
    validated_subgraphs: Vec<Subgraph<Validated>>,
) -> Result<ValidFederationSubgraphs, Vec<CompositionError>> {
    let mut map: BTreeMap<Arc<str>, ValidFederationSubgraph> = BTreeMap::new();
    let mut errors: Vec<CompositionError> = Vec::new();

    for sg in validated_subgraphs.into_iter() {
        match to_valid_federation_subgraph(sg) {
            Ok(vfs) => {
                // Create the Arc<str> key from the vfs.name (String)
                let key: Arc<str> = Arc::from(vfs.name.clone().into_boxed_str());
                if map.insert(key.clone(), vfs).is_some() {
                    errors.push(CompositionError::InternalError {
                        message: format!("duplicate subgraph name '{}'", key),
                    });
                }
            }
            Err(e) => errors.push(e),
        }
    }

    if errors.is_empty() {
        Ok(ValidFederationSubgraphs { subgraphs: map })
    } else {
        Err(errors)
    }
}
pub fn merge_subgraphs(
    validated_subgraphs: Vec<Subgraph<Validated>>,
) -> Result<Supergraph<Merged>, Vec<CompositionError>> {
    // Convert inputs to the merger's expected type
    let vfs = match vec_to_valid_federation_subgraphs(validated_subgraphs) {
        Ok(x) => x,
        Err(errs) => return Err(errs),
    };

    // Call the merger
    match merge_federation_subgraphs(vfs) {
        Ok(ms) => {
            // MergeSuccess { schema: Valid<Schema>, composition_hints: Vec<MergeWarning> }
            // Build Supergraph<Merged> from the returned Valid<Schema>
            let sg = Supergraph::<Merged>::new(ms.schema);
            // Optionally attach hints from ms.composition_hints into sg's hints if desired.
            Ok(sg)
        }
        Err(mf) => {
            // Convert MergeFailure.errors: Vec<MergeError> -> Vec<CompositionError>
            if !mf.errors.is_empty() {
                let errs: Vec<CompositionError> = mf
                    .errors
                    .into_iter()
                    .map(|merge_err| CompositionError::InternalError {
                        message: format!("{:?}", merge_err),
                    })
                    .collect();
                Err(errs)
            } else {
                // No structured errors — return a fallback internal error with hints if any
                Err(vec![CompositionError::InternalError {
                    message: format!(
                        "merge_federation_subgraphs failed with no errors; composition_hints: {:?}",
                        mf.composition_hints
                    ),
                }])
            }
        }
    }
}

pub fn post_merge_validations(
    supergraph: &Supergraph<Merged>,
) -> Result<(), Vec<CompositionError>> {
    let fed_schema = match ValidFederationSchema::new(supergraph.state.schema().clone()) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![CompositionError::InternalError {
                message: format!("failed to construct ValidFederationSchema: {:?}", e),
            }]);
        }
    };

    let mut errors: Vec<CompositionError> = Vec::new();

    if let Err(mut e) = validate_federation_directives_quick(&fed_schema) {
        errors.append(&mut e);
    }
    if let Err(mut e) = validate_no_conflicting_root_types_quick(&fed_schema) {
        errors.append(&mut e);
    }

    if let Err(mut e) = validate_field_ownership_and_references_quick(&fed_schema) {
        errors.append(&mut e);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Try to do a cheap validation of federation directives using SDL scanning.
///
/// This is a pragmatic, short-term validator. It will catch:
///  - @key without `fields`
///  - @key/@requires/@provides whose `fields` value fails a simple FieldSet parse
///
/// It will NOT correctly implement the full FieldSet grammar or ownership rules.
/// Use AST-based validators for production correctness later.
pub fn validate_federation_directives_quick(
    fed_schema: &ValidFederationSchema,
) -> Result<(), Vec<CompositionError>> {
    // 1) Get SDL from the schema. Adapt `get_sdl_from_valid_fed_schema` if your code exposes a printer.
    let sdl = match get_sdl_from_valid_fed_schema(fed_schema) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![CompositionError::InternalError {
                message: format!("could not obtain SDL from supergraph schema: {}", e),
            }]);
        }
    };

    // 2) Find type blocks and validate directives inside them
    let mut errors: Vec<CompositionError> = Vec::new();

    // A very small regex-free scanner: find "type NAME { ... }" blocks.
    // This is simple and won't handle every GraphQL construct (e.g., comments, complex directives across lines),
    // but is sufficient for quick checks.
    let mut chars = sdl.as_str();
    while let Some(type_idx) = chars.find("type ") {
        chars = &chars[type_idx + "type ".len()..];
        // read type name
        if let Some(rest) = chars.split_once('{') {
            let header = rest.0.trim();
            // header may contain "TypeName implements X & Y" or directives — take first token as name
            let type_name = header.split_whitespace().next().unwrap_or("<unknown>");

            // find matching closing brace for the block
            if let Some(mut depth_pos) = rest.1.find('}') {
                // crude: assume first '}' ends block. For nested braces we should track depth,
                // but GraphQL type bodies don't nest braces for fields (except for selection sets inside directives).
                let body = &rest.1[..depth_pos];

                // Now scan body for directives lines
                // Check @key usages that appear either on type header (we scanned header) or in body (extensions)
                // Quick check: Look for "@key(" in header and body and ensure `fields:` appears.
                for place in [&header, body] {
                    for key_pos in place.match_indices("@key") {
                        // look ahead for "fields"
                        let after = &place[key_pos.0..];
                        if !after.contains("fields") {
                            errors.push(CompositionError::InternalError {
                                message: format!(
                                    "@key on type '{}' missing `fields` argument",
                                    type_name
                                ),
                            });
                        } else {
                            // attempt to extract the fields string between parentheses if present
                            if let Some(opt_fields) = extract_directive_arg_str(after, "fields") {
                                if let Err(e) = parse_field_set_string(&opt_fields) {
                                    errors.push(CompositionError::InternalError {
                                        message: format!(
                                            "invalid FieldSet in @key on '{}': {}",
                                            type_name, e
                                        ),
                                    });
                                }
                            }
                        }
                    }

                    // check @requires and @provides occurrences in the body (field-level)
                    // We look for occurrences like "fieldName(...) @requires(fields: \"a b\")"
                    // crude approach: split body into lines and scan each line
                    for line in place.lines() {
                        if line.contains("@requires") {
                            if let Some(arg) = extract_directive_arg_str(line, "fields") {
                                if let Err(e) = parse_field_set_string(&arg) {
                                    errors.push(CompositionError::InternalError {
                                        message: format!(
                                            "invalid FieldSet in @requires on '{}': {}",
                                            type_name, e
                                        ),
                                    });
                                }
                            } else {
                                errors.push(CompositionError::InternalError {
                                    message: format!(
                                        "@requires on type '{}' has no `fields` argument",
                                        type_name
                                    ),
                                });
                            }
                        }
                        if line.contains("@provides") {
                            if let Some(arg) = extract_directive_arg_str(line, "fields") {
                                if let Err(e) = parse_field_set_string(&arg) {
                                    errors.push(CompositionError::InternalError {
                                        message: format!(
                                            "invalid FieldSet in @provides on '{}': {}",
                                            type_name, e
                                        ),
                                    });
                                }
                            } else {
                                errors.push(CompositionError::InternalError {
                                    message: format!(
                                        "@provides on type '{}' has no `fields` argument",
                                        type_name
                                    ),
                                });
                            }
                        }
                    }
                }

                // advance chars beyond this block
                chars = &rest.1[depth_pos + 1..];
            } else {
                // no closing brace found — bail
                break;
            }
        } else {
            break;
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Try multiple ways to get an SDL string from the ValidFederationSchema.
/// Adapt this function to call any schema-printer functions your repo exposes.
/// Returns Err(String) if none of the tried methods exist/succeed.
fn get_sdl_from_valid_fed_schema(fed_schema: &ValidFederationSchema) -> Result<String, String> {
    // Strategy A: if there's a crate-level printer you can call, uncomment and adapt:
    // return Ok(crate::schema::print_schema(fed_schema.schema().clone().into_inner()));

    // Strategy B: if Schema implements Display/ToString:
    // try to clone and call to_string (may or may not exist depending on Schema API).
    if let Ok(s) = std::panic::catch_unwind(|| {
        // If Valid<Schema> supports `.clone().into_inner()` and Schema implements ToString:
        let schema_val = fed_schema.schema().clone();
        let inner = schema_val.into_inner();
        inner.to_string()
    }) {
        return Ok(s);
    }

    // Could not produce SDL; return helpful message for you to adapt.
    Err("no known schema->SDL printer found; adapt get_sdl_from_valid_fed_schema to call your printer".to_string())
}

/// Extract the string value of a directive argument like fields: "a b c"
/// scans a short substring and returns the inner string if found.
/// Returns None if not found.
fn extract_directive_arg_str(src: &str, arg_name: &str) -> Option<String> {
    // look for e.g. fields: "a b c" or fields: 'a b c'
    if let Some(idx) = src.find(arg_name) {
        // find the colon after arg_name
        if let Some(colon_pos) = src[idx..].find(':') {
            let after_colon = &src[idx + colon_pos + 1..];
            // find first quote char
            if let Some(first_q_pos) = after_colon.find('"') {
                let rest = &after_colon[first_q_pos + 1..];
                if let Some(second_q_pos) = rest.find('"') {
                    return Some(rest[..second_q_pos].to_string());
                }
            } else if let Some(first_sq) = after_colon.find('\'') {
                let rest = &after_colon[first_sq + 1..];
                if let Some(second_sq) = rest.find('\'') {
                    return Some(rest[..second_sq].to_string());
                }
            } else {
                // no quotes; maybe unquoted token(s) — take up to whitespace or ')'
                let trimmed = after_colon.trim_start();
                let mut end = trimmed.len();
                for (i, c) in trimmed.char_indices() {
                    if c.is_whitespace() || c == ')' || c == ',' {
                        end = i;
                        break;
                    }
                }
                if end > 0 {
                    return Some(trimmed[..end].trim().to_string());
                }
            }
        }
    }
    None
}

/// Simple FieldSet parser used above (same as earlier).
fn parse_field_set_string(input: &str) -> Result<Vec<Vec<String>>, String> {
    let mut result: Vec<Vec<String>> = Vec::new();
    for token in input.split_whitespace() {
        if token.is_empty() {
            continue;
        }
        if token
            .chars()
            .any(|c| c == '(' || c == ')' || c == '{' || c == '}')
        {
            return Err("unsupported FieldSet token (contains braces/paren)".to_string());
        }
        let path: Vec<String> = token.split('.').map(|s| s.to_string()).collect();
        if path.is_empty() {
            return Err("empty field path".to_string());
        }
        result.push(path);
    }
    Ok(result)
}

/// Quick heuristic: detect conflicting definitions for the same type name.
/// It compares the textual bodies (whitespace-normalized). If type `Foo` appears
/// more than once with different bodies we consider that a conflict.
pub fn validate_no_conflicting_root_types_quick(
    fed_schema: &ValidFederationSchema,
) -> Result<(), Vec<CompositionError>> {
    let sdl = match get_sdl_from_valid_fed_schema(fed_schema) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![CompositionError::InternalError {
                message: format!("could not obtain SDL for conflict check: {}", e),
            }]);
        }
    };

    // Map: type_name -> normalized body string (first occurrence)
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut errors: Vec<CompositionError> = Vec::new();

    // crude parser: find "type <Name> { ... }" blocks and capture the inside as body
    let mut rest = sdl.as_str();
    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        // get up to next '{'
        if let Some((header, after_brace)) = rest.split_once('{') {
            let header = header.trim();
            let type_name = header.split_whitespace().next().unwrap_or("").to_string();
            // find the matching '}' for this block (simple first '}' in remainder)
            if let Some(body_end_pos) = after_brace.find('}') {
                let body = &after_brace[..body_end_pos];
                let normalized = normalize_whitespace(body);
                if let Some(existing) = seen.get(&type_name) {
                    if existing != &normalized {
                        errors.push(CompositionError::InternalError {
                            message: format!("conflicting definitions for type '{}'", type_name),
                        });
                    }
                } else {
                    seen.insert(type_name, normalized);
                }
                rest = &after_brace[body_end_pos + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Normalize whitespace in a string: collapse consecutive whitespace into a single space and trim.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// Quick heuristic for field ownership & references:
/// - builds a map of type -> fields (from SDL)
/// - for each occurrence of '@requires(fields: "...")' attached to a field
///   in a type, ensure the referenced fields exist on the same type.
pub fn validate_field_ownership_and_references_quick(
    fed_schema: &ValidFederationSchema,
) -> Result<(), Vec<CompositionError>> {
    let sdl = match get_sdl_from_valid_fed_schema(fed_schema) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![CompositionError::InternalError {
                message: format!("could not obtain SDL for ownership check: {}", e),
            }]);
        }
    };

    // First pass: build type -> set of field names
    let mut types_map: HashMap<String, HashSet<String>> = HashMap::new();

    let mut rest = sdl.as_str();
    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        if let Some((header, after_brace)) = rest.split_once('{') {
            let header = header.trim();
            let type_name = header.split_whitespace().next().unwrap_or("").to_string();
            if let Some(body_end_pos) = after_brace.find('}') {
                let body = &after_brace[..body_end_pos];
                let mut fields: HashSet<String> = HashSet::new();
                for line in body.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    // field lines generally start with the field name: e.g. "id: ID!" or "user(id: ID): User @requires(...)"
                    // take characters until whitespace, '(' or ':' as the field name
                    let field_name = trimmed
                        .split_whitespace()
                        .next()
                        .map(|tok| tok.split(&[':', '('][..]).next().unwrap_or(tok))
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !field_name.is_empty() {
                        fields.insert(field_name);
                    }
                }
                types_map.insert(type_name, fields);
                rest = &after_brace[body_end_pos + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Second pass: find @requires usages and verify referenced fields exist on same type
    let mut errors: Vec<CompositionError> = Vec::new();
    let mut rest = sdl.as_str();
    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        if let Some((header, after_brace)) = rest.split_once('{') {
            let header = header.trim();
            let type_name = header.split_whitespace().next().unwrap_or("").to_string();
            if let Some(body_end_pos) = after_brace.find('}') {
                let body = &after_brace[..body_end_pos];

                for line in body.lines() {
                    if line.contains("@requires") {
                        let trimmed = line.trim();
                        // field attached to this directive: first token
                        let field_token = trimmed
                            .split_whitespace()
                            .next()
                            .map(|tok| tok.split(&[':', '('][..]).next().unwrap_or(tok))
                            .unwrap_or("")
                            .to_string();
                        // extract the fields arg string from the directive occurrence
                        if let Some(fields_str) = extract_directive_arg_str(trimmed, "fields") {
                            match parse_field_set_string(&fields_str) {
                                Ok(paths) => {
                                    for path in paths {
                                        let referenced_field = &path[0];
                                        let type_fields = types_map.get(&type_name);
                                        if let Some(set) = type_fields {
                                            if !set.contains(referenced_field) {
                                                errors.push(CompositionError::InternalError {
                                                    message: format!(
                                                        "@requires on '{}.{}' references unknown field '{}'",
                                                        type_name, field_token, referenced_field
                                                    ),
                                                });
                                            }
                                        } else {
                                            errors.push(CompositionError::InternalError {
                                                message: format!(
                                                    "@requires on '{}.{}' references '{}' but type '{}' was not found in schema",
                                                    type_name, field_token, referenced_field, type_name
                                                ),
                                            });
                                        }
                                    }
                                }
                                Err(e) => {
                                    errors.push(CompositionError::InternalError {
                                        message: format!(
                                            "invalid FieldSet in @requires on '{}.{}': {}",
                                            type_name, field_token, e
                                        ),
                                    });
                                }
                            }
                        } else {
                            errors.push(CompositionError::InternalError {
                                message: format!(
                                    "@requires on '{}.{}' missing `fields` argument",
                                    type_name, field_token
                                ),
                            });
                        }
                    }

                    // optional: quick check for @provides (ensure referenced fields exist somewhere)
                    if line.contains("@provides") {
                        if let Some(fields_str) = extract_directive_arg_str(line, "fields") {
                            if let Err(e) = parse_field_set_string(&fields_str) {
                                errors.push(CompositionError::InternalError {
                                    message: format!(
                                        "invalid FieldSet in @provides on '{}': {}",
                                        type_name, e
                                    ),
                                });
                            }
                        } else {
                            errors.push(CompositionError::InternalError {
                                message: format!(
                                    "@provides in type '{}' missing `fields` argument",
                                    type_name
                                ),
                            });
                        }
                    }
                }

                rest = &after_brace[body_end_pos + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}
