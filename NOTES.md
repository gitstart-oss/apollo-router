# Goal

Port the Node compose logic (composition-js/src/compose.ts) into the Rust crate (apollo-federation/src/composition/mod.rs) and implement the composition pipeline end-to-end in Rust:

expand subgraphs

upgrade subgraphs (if necessary)

validate subgraphs

pre-merge validations

merge subgraphs (call existing merger)

post-merge validations

optional satisfiability checks

This document summarizes what I implemented, why, key design choices, limitations, and suggested next steps.

What I implemented

File changed: apollo-federation/src/composition/mod.rs

Implemented compose_with_options and compose (mirrors JS compose with a run_satisfiability toggle).

Implemented subgraph lifecycle helpers:

expand_subgraphs

validate_subgraphs

pre_merge_validations

merge_subgraphs (delegates to merge_federation_subgraphs)

post_merge_validations

Ported several SDL-based validators from the JS implementation:

validate_directive_field_sets — @key, @requires, @provides FieldSet syntax + existence checks

validate_no_conflicting_root_types_quick and validate_no_conflicting_root_types_enhanced

validate_field_ownership_and_references_quick

validate_join_field_type_consistency_quick

validate_key_fields_exist

Added utilities:

get_sdl_from_valid_fed_schema — best-effort SDL extraction (fallbacks to to_string()).

extract_directive_arg_str / extract_directive_arg_token — simple directive argument parsing.

parse_field_set_string — small FieldSet parser.

get_type_fields_map_from_sdl — map type -> field names for quick checks.

Key design decisions & rationale

Reuse the existing merger
The Rust code delegates merging to merge_federation_subgraphs. This preserves the semantics the JS implementation relies on (join metadata, graph enum handling, correct handling of extensions and collisions).

SDL-based quick validators first
Ported SDL heuristics replicate the JS behavior quickly and match tests. AST-based validators are safer/stronger and should be used later when stable AST access is preferred.

Satisfiability toggle
compose_with_options supports running or skipping satisfiability, matching the JS API (runSatisfiability).

Aggregate errors
Post-merge validators collect and return all found errors as Vec<CompositionError>, matching existing Rust error conventions.

Safe SDL extraction
get_sdl_from_valid_fed_schema uses to_string() inside catch_unwind as a fallback. Replace with a canonical printer if available.

Important note: SupergraphBuilder & naive concatenation

I added an example SupergraphBuilder in the codebase for convenience, but do not use naive concatenation of subgraph SDLs for composition in production. The merger (merge_federation_subgraphs) performs essential federation-specific work:

Computes @join\_\_\* metadata

Constructs graph enum and subgraph mapping

Handles extensions and name conflicts

Produces structured merge errors/hints

Naive concatenation can produce superficially-valid SDL that lacks correct federation semantics. If you need a builder pattern, prefer a builder that collects validated Subgraph<Validated> instances and calls merge_subgraphs.

Example safe builder pattern:

pub struct SupergraphBuilder {
validated_subgraphs: Vec<Subgraph<Validated>>,
}

impl SupergraphBuilder {
pub fn add_validated_subgraph(&mut self, sg: Subgraph<Validated>) {
self.validated_subgraphs.push(sg);
}

    pub fn finish(self) -> Result<Supergraph<Merged>, Vec<CompositionError>> {
        merge_subgraphs(self.validated_subgraphs)
    }

}

Known limitations & TODOs

SDL printing dependency — get_sdl_from_valid_fed_schema currently uses to_string(). Replace with canonical schema-printer (e.g., print_sdl) if available.

Validators — currently SDL-based heuristics. When possible, switch to AST-based validators (more robust).

FieldSet parser — minimal implementation. If your schemas use nested FieldSets or advanced features, replace with a full parser.

Hints — JS returns merge + satisfiability hints. The Rust implementation notes where to capture those; wiring them into Supergraph<Satisfiable>::hints() is left as a follow-up.

Error types — some checks return InternalError or TypeDefinitionInvalid — ensure test expectations align with these error variants/messages.
