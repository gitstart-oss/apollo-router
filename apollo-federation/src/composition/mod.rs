mod satisfiability;
use apollo_compiler::Name;
use apollo_compiler::ast::Value;
use apollo_compiler::schema::ExtendedType;

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
use crate::supergraph::CompositionHint;
pub use crate::supergraph::Merged;
pub use crate::supergraph::Satisfiable;
pub use crate::supergraph::Supergraph;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::vec;
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

fn validate_composition_options(_options: &CompositionOptions) -> Result<(), CompositionError> {
    // In the JS implementation there's a guard against "list_upgrade" being present in
    // TODO: FED-570, we might want to add similar guards here if we add more options.
    // validating it here. For now, it's a no-op.
    Ok(())
}

pub fn compose_with_options(
    subgraphs: Vec<Subgraph<Initial>>,
    options: CompositionOptions,
) -> Result<Supergraph<Satisfiable>, Vec<CompositionError>> {
    if let Err(e) = validate_composition_options(&options) {
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

/// For Convenience: preserving prior default behavior (runs satisfiability).
pub fn compose(
    subgraphs: Vec<Subgraph<Initial>>,
) -> Result<Supergraph<Satisfiable>, Vec<CompositionError>> {
    compose_with_options(subgraphs, CompositionOptions::default())
}

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

/// ---------------------------------------------------------------------------
/// --- Implementations -------
/// ---------------------------------------------------------------------------
///


pub fn pre_merge_validations(
    subgraphs: &[Subgraph<Validated>],
) -> Result<(), Vec<CompositionError>> {
    let mut errors: Vec<CompositionError> = Vec::new();

    // 1) Service name checks (empty / duplicate)
    let mut seen_names: HashSet<String> = HashSet::new();
    for sg in subgraphs {
        let name = sg.name.trim();
        if name.is_empty() {
            errors.push(CompositionError::EmptySubgraphName);
            continue;
        }
        if !seen_names.insert(name.to_string()) {
            errors.push(CompositionError::DuplicateSubgraphName {
                name: name.to_string(),
            });
        }
    }

    // Cross-subgraph aggregations
    let mut type_kinds: HashMap<String, HashSet<&'static str>> = HashMap::new();
    let mut dir_defs: HashMap<String, Vec<(bool, Vec<String>)>> = HashMap::new(); // @name -> [(repeatable, locations)]
    let mut scalar_urls: HashMap<String, HashSet<String>> = HashMap::new(); // scalar -> {url,...}

    // 2) Per-subgraph validations
    for sg in subgraphs {
        {
            let schema = sg.schema().schema();
            for (_k, def) in schema.directive_definitions.iter() {
                let name = def.name.to_string();
                let repeatable = def.repeatable;
                let mut locs: Vec<String> =
                    def.locations.iter().map(|loc| loc.to_string()).collect();
                locs.sort();
                dir_defs.entry(name).or_default().push((repeatable, locs));
            }
        }
        // 2c (AST-based): collect scalar @specifiedBy(url: "...") values
        {
            let schema = sg.schema().schema();
            // Name("url") once so we don't allocate repeatedly
            let url_name: Name = "url".try_into().expect("valid name");
            for (_name, tdef) in schema.types.iter() {
                if let ExtendedType::Scalar(scalar) = tdef {
                    let scalar_name = scalar.name.to_string();

                    for d in &scalar.directives {
                        if d.name.as_str() != "specifiedBy" {
                            continue;
                        }

                        // Prefer using the helper: argument_by_name(&Name, &Schema)
                        if let Ok(val) = d.argument_by_name(&url_name, schema) {
                            if let Some(url) = val.as_str() {
                                scalar_urls
                                    .entry(scalar_name.clone())
                                    .or_default()
                                    .insert(url.to_string());
                                continue;
                            }
                        }

                        // Fallback: walk arguments directly for older/newer Value layouts
                        for arg in &d.arguments {
                            if arg.name.as_str() == "url" {
                                match &*arg.value {
                                    // Some versions use Value::String
                                    Value::String(s) => {
                                        scalar_urls
                                            .entry(scalar_name.clone())
                                            .or_default()
                                            .insert(s.clone());
                                    }
                                    // Others expose an as_str() helper
                                    v => {
                                        if let Some(s) = v.as_str() {
                                            scalar_urls
                                                .entry(scalar_name.clone())
                                                .or_default()
                                                .insert(s.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Trying SDL (kept for other quick text-based heuristics)
        let sdl = match subgraph_sdl(sg) {
            Ok(s) => s,
            Err(e) => {
                errors.push(CompositionError::InternalError {
                    message: format!("failed to obtain SDL for subgraph '{}': {}", sg.name, e),
                });
                continue;
            }
        };

        // 2a) Type-kind collection (object / interface / enum / scalar / union / input)
        for (tok, label) in [
            ("type ", "Object"),
            ("interface ", "Interface"),
            ("enum ", "Enum"),
            ("scalar ", "Scalar"),
            ("union ", "Union"),
            ("input ", "InputObject"),
        ] {
            let mut rest = sdl.as_str();
            while let Some(i) = rest.find(tok) {
                rest = &rest[i + tok.len()..];
                let ty = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches('{')
                    .to_string();
                if !ty.is_empty() && !ty.starts_with("__") {
                    type_kinds.entry(ty).or_default().insert(label);
                }
            }
        }

        // 2b (SDL-based too): capture directive definitions found in text (kept for completeness)
        for line in sdl
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("directive @"))
        {
            let name = line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .trim_start_matches('@')
                .to_string();
            let repeatable = line.contains(" repeatable ");
            let locs: Vec<String> = line
                .split(" on ")
                .nth(1)
                .map(|s| s.split('|').map(|t| t.trim().to_string()).collect())
                .unwrap_or_else(Vec::new);
            dir_defs.entry(name).or_default().push((repeatable, locs));
        }

        // 2d) Federation field-set sanity for this subgraph (SDL quick pass)
        let type_fields = get_type_fields_map_from_sdl(&sdl);
        let mut rest = sdl.as_str();
        while let Some(pos) = rest.find("type ") {
            rest = &rest[pos + "type ".len()..];
            if let Some((header, after_brace)) = rest.split_once('{') {
                let type_name = header
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or("<unknown>");

                let body_end = match after_brace.find('}') {
                    Some(p) => p,
                    None => break,
                };
                let body = &after_brace[..body_end];

                // @key on the type header
                for kpos in header.match_indices("@key") {
                    let after = &header[kpos.0..];
                    if let Some(fields_str) = extract_directive_arg_str(after, "fields") {
                        if let Err(e) = parse_field_set_string(&fields_str) {
                            errors.push(CompositionError::InvalidFieldSet {
                                directive: "key".into(),
                                type_name: type_name.to_string(),
                                details: e,
                            });
                        } else if let Some(fields) = type_fields.get(type_name) {
                            for token in fields_str.split_whitespace() {
                                let fname = token.split('.').next().unwrap_or("").trim();
                                if !fname.is_empty() && !fields.contains(fname) {
                                    errors.push(CompositionError::UnknownFieldInFieldSet {
                                        directive: "key".into(),
                                        type_name: type_name.to_string(),
                                        field: fname.to_string(),
                                    });
                                }
                            }
                        }
                    } else {
                        errors.push(CompositionError::InvalidFieldSet {
                            directive: "key".into(),
                            type_name: type_name.to_string(),
                            details: "missing `fields` argument".into(),
                        });
                    }
                }

                // @requires / @provides on fields
                for line in body.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }

                    if trimmed.contains("@requires") {
                        let field_name = trimmed
                            .split(|c| c == ':' || c == '(')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        match extract_directive_arg_str(trimmed, "fields") {
                            Some(arg) => match parse_field_set_string(&arg) {
                                Ok(paths) => {
                                    if let Some(fields) = type_fields.get(type_name) {
                                        for path in paths {
                                            let ref_field = &path[0];
                                            if !fields.contains(ref_field) {
                                                errors.push(
                                                    CompositionError::UnknownFieldInFieldSet {
                                                        directive: "requires".into(),
                                                        type_name: type_name.to_string(),
                                                        field: ref_field.clone(),
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => errors.push(CompositionError::InvalidFieldSet {
                                    directive: "requires".into(),
                                    type_name: type_name.to_string(),
                                    details: format!("on '{}': {}", field_name, e),
                                }),
                            },
                            None => errors.push(CompositionError::InvalidFieldSet {
                                directive: "requires".into(),
                                type_name: type_name.to_string(),
                                details: format!("on '{}' missing `fields` argument", field_name),
                            }),
                        }
                    }

                    if trimmed.contains("@provides") {
                        match extract_directive_arg_str(trimmed, "fields") {
                            Some(arg) => {
                                if let Err(e) = parse_field_set_string(&arg) {
                                    errors.push(CompositionError::InvalidFieldSet {
                                        directive: "provides".into(),
                                        type_name: type_name.to_string(),
                                        details: e,
                                    });
                                }
                            }
                            None => errors.push(CompositionError::InvalidFieldSet {
                                directive: "provides".into(),
                                type_name: type_name.to_string(),
                                details: "missing `fields` argument".into(),
                            }),
                        }
                    }
                }

                rest = &after_brace[body_end + 1..];
            } else {
                break;
            }
        }
    }

    // 3) Cross-subgraph conflict reporting

    for (ty, kinds) in type_kinds {
        if kinds.len() > 1 {
            errors.push(CompositionError::TypeKindConflict {
                type_name: ty,
                kinds: kinds.into_iter().map(|s| s.to_string()).collect(),
            });
        }
    }

    // Directive definition conflicts
    for (dir, defs) in dir_defs {
        let mut repeatables = HashSet::new();
        let mut loc_sets: HashSet<Vec<String>> = HashSet::new();
        for (rep, mut locs) in defs {
            repeatables.insert(rep);
            locs.sort();
            loc_sets.insert(locs);
        }
        if repeatables.len() > 1 {
            errors.push(CompositionError::DirectiveRepeatableConflict {
                directive: dir.clone(),
            });
        }
        if loc_sets.len() > 1 {
            errors.push(CompositionError::DirectiveLocationsConflict { directive: dir });
        }
    }

    // Scalar specifiedBy URL mismatches (now reliably populated by the AST step)
    for (scalar, urls) in scalar_urls {
        if urls.len() > 1 {
            errors.push(CompositionError::ScalarSpecifiedByUrlConflict {
                scalar,
                urls: urls.into_iter().collect(),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn subgraph_sdl(sg: &Subgraph<Validated>) -> Result<String, CompositionError> {
    Ok(sg.original_sdl().to_owned())
}

pub fn merge_subgraphs(
    validated_subgraphs: Vec<Subgraph<Validated>>,
) -> Result<MergeOutput, Vec<CompositionError>> {
    let vfs = vec_to_valid_federation_subgraphs(validated_subgraphs)?;
    match merge_federation_subgraphs(vfs) {
        Ok(ms) => {
            let sg = Supergraph::<Merged>::new(ms.schema);
            Ok(MergeOutput {
                supergraph: sg,
                hints: ms.composition_hints, 
            })
        }
        Err(mf) => {
            if !mf.errors.is_empty() {
                Err(mf
                    .errors
                    .into_iter()
                    .map(|e| CompositionError::TypeDefinitionInvalid {
                        message: format!("{:?}", e),
                    })
                    .collect())
            } else {
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
    // directive / FieldSet validation (quick SDL based)
    if let Err(mut e) = validate_directive_field_sets(&fed_schema) {
        errors.append(&mut e);
    }
    // conflicting root type heuristics
    if let Err(mut e) = validate_no_conflicting_root_types_quick(&fed_schema) {
        errors.append(&mut e);
    }
    if let Err(mut e) = validate_no_conflicting_root_types_enhanced(&fed_schema) {
        errors.append(&mut e);
    }
    // field ownership / @requires/@provides checks
    if let Err(mut e) = validate_field_ownership_and_references_quick(&fed_schema) {
        errors.append(&mut e);
    }
    // check join field type consistency across graphs (quick SDL heuristic)
    if let Err(mut e) = validate_join_field_type_consistency_quick(&fed_schema) {
        errors.append(&mut e);
    }
    // key fields existence check (ensures @key references existing fields)
    if let Err(mut e) = validate_key_fields_exist(&fed_schema) {
        errors.append(&mut e);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}




/// ---------------------------------------------------------------------------
/// --- Validators / helpers (SDL-based should look into AST) -------
/// ---------------------------------------------------------------------------
///
///
///
///
/// --- Helpers to convert the validated Subgraph list into ValidFederationSubgraphs expected by the merger ---
fn to_valid_federation_subgraph(
    sg: Subgraph<Validated>,
) -> Result<ValidFederationSubgraph, CompositionError> {
    let validated_schema = sg.validated_schema().clone();
    let name_string = sg.name;
    let url_string = sg.url;
    Ok(ValidFederationSubgraph {
        name: name_string.clone(),
        url: url_string,
        schema: validated_schema,
    })
}

fn vec_to_valid_federation_subgraphs(
    validated_subgraphs: Vec<Subgraph<Validated>>,
) -> Result<ValidFederationSubgraphs, Vec<CompositionError>> {
    let mut map: BTreeMap<Arc<str>, ValidFederationSubgraph> = BTreeMap::new();
    let mut errors: Vec<CompositionError> = Vec::new();

    for sg in validated_subgraphs.into_iter() {
        match to_valid_federation_subgraph(sg) {
            Ok(vfs) => {
                // Create Arc<str> key and insert
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

/// Return a mapping: type name -> set of field names for quick existence checks.
fn get_type_fields_map_from_sdl(sdl: &str) -> HashMap<String, HashSet<String>> {
    let mut type_fields: HashMap<String, HashSet<String>> = HashMap::new();
    let mut rest = sdl;

    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        if let Some((header, after_brace)) = rest.split_once('{') {
            let type_name = header
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if let Some(body_end_pos) = after_brace.find('}') {
                let body = &after_brace[..body_end_pos];
                let mut fields = HashSet::new();
                for line in body.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') || !trimmed.contains(':') {
                        continue;
                    }
                    if let Some((field_name, _)) = trimmed.split_once(':') {
                        let field_name = field_name.trim().to_string();
                        if !field_name.is_empty() {
                            fields.insert(field_name);
                        }
                    }
                }
                type_fields.insert(type_name, fields);
                rest = &after_brace[body_end_pos + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    type_fields
}

/// Validate @key / @requires / @provides directive field sets and field existence (SDL-based).
pub fn validate_directive_field_sets(
    fed_schema: &ValidFederationSchema,
) -> Result<(), Vec<CompositionError>> {
    let sdl = match get_sdl_from_valid_fed_schema(fed_schema) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![CompositionError::InternalError {
                message: format!("could not obtain SDL from supergraph schema: {}", e),
            }]);
        }
    };

    let type_fields = get_type_fields_map_from_sdl(&sdl);
    let mut errors: Vec<CompositionError> = Vec::new();

    let mut chars = &sdl[..];
    while let Some(type_idx) = chars.find("type ") {
        chars = &chars[type_idx + "type ".len()..];
        if let Some(rest) = chars.split_once('{') {
            let header = rest.0.trim();
            let type_name = header.split_whitespace().next().unwrap_or("<unknown>");

            if let Some(depth_pos) = rest.1.find('}') {
                let body = &rest.1[..depth_pos];

                for place in [&header, body] {
                    for key_pos in place.match_indices("@key") {
                        let after = &place[key_pos.0..];
                        if !after.contains("fields") {
                            errors.push(CompositionError::InternalError {
                                message: format!(
                                    "@key on type '{}' missing `fields` argument",
                                    type_name
                                ),
                            });
                        } else {
                            if let Some(opt_fields) = extract_directive_arg_str(after, "fields") {
                                if let Err(e) = parse_field_set_string(&opt_fields) {
                                    errors.push(CompositionError::InternalError {
                                        message: format!(
                                            "invalid FieldSet in @key on '{}': {}",
                                            type_name, e
                                        ),
                                    });
                                }
                                if let Some(fields) = type_fields.get(type_name) {
                                    for token in opt_fields.split_whitespace() {
                                        if token.is_empty() {
                                            continue;
                                        }
                                        let field_name =
                                            token.split('.').next().unwrap_or("").trim();
                                        if !field_name.is_empty() && !fields.contains(field_name) {
                                            errors.push(CompositionError::InternalError {
                                                message: format!(
                                                    "field '{}' in @key on '{}' does not exist",
                                                    field_name, type_name
                                                ),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }

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
                                if let Some(fields) = type_fields.get(type_name) {
                                    for token in arg.split_whitespace() {
                                        if token.is_empty() {
                                            continue;
                                        }
                                        let field_name =
                                            token.split('.').next().unwrap_or("").trim();
                                        if !field_name.is_empty() && !fields.contains(field_name) {
                                            errors.push(CompositionError::InternalError {
                                                message: format!("field '{}' in @requires on '{}' does not exist", field_name, type_name),
                                            });
                                        }
                                    }
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

                chars = &rest.1[depth_pos + 1..];
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

/// Try multiple ways to get an SDL string from the ValidFederationSchema.
/// Returns Err(String) if none of the tried methods exist/succeed.
fn get_sdl_from_valid_fed_schema(fed_schema: &ValidFederationSchema) -> Result<String, String> {
    // Preferred: call a crate-level printer if available (uncomment & adapt)
    // return Ok(crate::schema::print_schema(fed_schema.schema().clone().into_inner()));

    // Fallback: try to_string via panic/catch_unwind (best-effort).
    if let Ok(s) = std::panic::catch_unwind(|| {
        let schema_val = fed_schema.schema().clone();
        let inner = schema_val.into_inner();
        inner.to_string()
    }) {
        return Ok(s);
    }

    Err("no known schema->SDL printer found; adapt get_sdl_from_valid_fed_schema to call your printer".to_string())
}

/// Extract the string value of a directive argument like fields: "a b c"
fn extract_directive_arg_str(src: &str, arg_name: &str) -> Option<String> {
    if let Some(idx) = src.find(arg_name) {
        if let Some(colon_pos) = src[idx..].find(':') {
            let after_colon = &src[idx + colon_pos + 1..];
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

/// Extract an unquoted token argument value like graph: S1
fn extract_directive_arg_token(src: &str, arg_name: &str) -> Option<String> {
    if let Some(idx) = src.find(arg_name) {
        if let Some(colon_pos) = src[idx..].find(':') {
            let after_colon = &src[idx + colon_pos + 1..];
            let trimmed = after_colon.trim_start();
            let mut end = trimmed.len();
            for (i, c) in trimmed.char_indices() {
                if c == ',' || c == ')' || c.is_whitespace() {
                    end = i;
                    break;
                }
            }
            if end > 0 {
                let tok = trimmed[..end]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !tok.is_empty() {
                    return Some(tok);
                }
            }
        }
    }
    None
}

/// Simple FieldSet parser used above (very small subset).
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

/// Quick heuristic to detect conflicting type definitions across subgraphs.
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

    let mut seen: HashMap<String, String> = HashMap::new();
    let mut errors: Vec<CompositionError> = Vec::new();
    let mut rest = sdl.as_str();

    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        if let Some((header, after_brace)) = rest.split_once('{') {
            let header = header.trim();
            let type_name = header.split_whitespace().next().unwrap_or("").to_string();
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

/// Enhanced conflicting-field-type detection across occurrences in the SDL.
pub fn validate_no_conflicting_root_types_enhanced(
    fed_schema: &ValidFederationSchema,
) -> Result<(), Vec<CompositionError>> {
    let sdl = match get_sdl_from_valid_fed_schema(fed_schema) {
        Ok(s) => s,
        Err(e) => return Err(vec![CompositionError::InternalError { message: e }]),
    };

    let mut type_fields: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut errors: Vec<CompositionError> = Vec::new();
    let mut rest = &sdl[..];

    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        if let Some((header, after_brace)) = rest.split_once('{') {
            let type_name = header
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            let type_name_clone = type_name.clone();
            if let Some(body_end_pos) = after_brace.find('}') {
                let body = &after_brace[..body_end_pos];
                for line in body.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') || !trimmed.contains(':') {
                        continue;
                    }
                    if let Some((field_name, field_type_raw)) = trimmed.split_once(':') {
                        let field_name = field_name.trim().to_string();
                        let field_type = field_type_raw
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if let Some(existing_fields) = type_fields.get_mut(&type_name_clone) {
                            if let Some(existing_type) = existing_fields.get(&field_name) {
                                if existing_type != &field_type {
                                    errors.push(CompositionError::InternalError {
                                        message: format!(
                                            "conflicting types for field '{}.{}': {} vs {}",
                                            type_name_clone, field_name, existing_type, field_type
                                        ),
                                    });
                                }
                            } else {
                                existing_fields.insert(field_name, field_type);
                            }
                        } else {
                            let mut fields = HashMap::new();
                            fields.insert(field_name, field_type);
                            type_fields.insert(type_name_clone.clone(), fields);
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

/// Normalize whitespace in a string: collapse consecutive whitespace into a single space and trim.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Quick heuristic for field ownership & references (@requires / @provides).
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
                        let field_token = trimmed
                            .split_whitespace()
                            .next()
                            .map(|tok| tok.split(&[':', '('][..]).next().unwrap_or(tok))
                            .unwrap_or("")
                            .to_string();
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

/// Quick check that join__fields with per-graph `type:` argument are consistent across graphs.
pub fn validate_join_field_type_consistency_quick(
    fed_schema: &ValidFederationSchema,
) -> Result<(), Vec<CompositionError>> {
    let sdl = match get_sdl_from_valid_fed_schema(fed_schema) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![CompositionError::InternalError {
                message: format!("could not obtain SDL for join__field check: {}", e),
            }]);
        }
    };

    let mut field_types_by_field: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut errors: Vec<CompositionError> = Vec::new();
    let mut rest = &sdl[..];

    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        if let Some((header, after_brace)) = rest.split_once('{') {
            let type_name = header
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if let Some(body_end_pos) = after_brace.find('}') {
                let body = &after_brace[..body_end_pos];
                for line in body.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    let field_name = trimmed
                        .split_whitespace()
                        .next()
                        .map(|tok| tok.split(&[':', '('][..]).next().unwrap_or(tok))
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if field_name.is_empty() {
                        continue;
                    }

                    let mut search = trimmed;
                    while let Some(jpos) = search.find("@join__field") {
                        let sub = &search[jpos..];
                        let graph_opt = extract_directive_arg_token(sub, "graph");
                        let type_opt = extract_directive_arg_str(sub, "type");

                        if let Some(graph) = graph_opt {
                            let type_str = type_opt.unwrap_or_else(|| "<unspecified>".to_string());
                            let key = format!("{}.{}", type_name, field_name);
                            field_types_by_field
                                .entry(key.clone())
                                .or_insert_with(HashMap::new)
                                .insert(graph.clone(), type_str.clone());
                        }

                        if jpos + "@join__field".len() >= search.len() {
                            break;
                        }
                        search = &search[jpos + "@join__field".len()..];
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

    for (field_key, graph_map) in field_types_by_field.into_iter() {
        let mut types_seen: HashSet<String> = HashSet::new();
        for (_graph, t) in graph_map.iter() {
            types_seen.insert(t.clone());
        }
        if types_seen.len() > 1 {
            let mut per_graph: Vec<String> = graph_map
                .into_iter()
                .map(|(g, t)| format!("{}: {}", g, t))
                .collect();
            per_graph.sort();
            errors.push(CompositionError::InternalError {
                message: format!(
                    "inconsistent per-graph types for '{}': {}",
                    field_key,
                    per_graph.join(", ")
                ),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate that @key referenced fields actually exist (SDL based).
pub fn validate_key_fields_exist(
    fed_schema: &ValidFederationSchema,
) -> Result<(), Vec<CompositionError>> {
    let sdl = match get_sdl_from_valid_fed_schema(fed_schema) {
        Ok(s) => s,
        Err(e) => {
            return Err(vec![CompositionError::InternalError {
                message: format!("could not obtain SDL for field existence check: {}", e),
            }]);
        }
    };

    let type_fields = get_type_fields_map_from_sdl(&sdl);
    let mut errors: Vec<CompositionError> = Vec::new();
    let mut rest = &sdl[..];

    while let Some(pos) = rest.find("type ") {
        rest = &rest[pos + "type ".len()..];
        if let Some((header, after_brace)) = rest.split_once('{') {
            let type_name = header
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if let Some(body_end_pos) = after_brace.find('}') {
                let body = &after_brace[..body_end_pos];

                for key_pos in header.match_indices("@key") {
                    let after = &header[key_pos.0..];
                    if let Some(fields_str) = extract_directive_arg_str(after, "fields") {
                        for token in fields_str.split_whitespace() {
                            if token.is_empty() {
                                continue;
                            }
                            let field_name = token.split('.').next().unwrap_or("").trim();
                            if let Some(fields) = type_fields.get(&type_name) {
                                if !fields.contains(field_name) {
                                    errors.push(CompositionError::InternalError {
                                        message: format!(
                                            "field '{}' in @key on '{}' does not exist",
                                            field_name, type_name
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }

                for line in body.lines() {
                    let trimmed = line.trim();
                    for key_pos in trimmed.match_indices("@key") {
                        let after = &trimmed[key_pos.0..];
                        if let Some(fields_str) = extract_directive_arg_str(after, "fields") {
                            for token in fields_str.split_whitespace() {
                                if token.is_empty() {
                                    continue;
                                }
                                let field_name = token.split('.').next().unwrap_or("").trim();
                                if let Some(fields) = type_fields.get(&type_name) {
                                    if !fields.contains(field_name) {
                                        errors.push(CompositionError::InternalError {
                                            message: format!(
                                                "field '{}' in @key on '{}' does not exist",
                                                field_name, type_name
                                            ),
                                        });
                                    }
                                }
                            }
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
