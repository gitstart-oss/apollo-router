
# Federation Composition Port (Node → Rust)

This document summarizes the end-to-end composition pipeline now implemented in `apollo-federation/src/composition/mod.rs`, the rationale behind key decisions, what’s still missing, and how the test suite exercises the behavior.

---

## Goals

Port the Node `composition-js/src/compose.ts` logic into Rust and wire up a full pipeline:

1) **Expand subgraphs**  
2) **Upgrade subgraphs** (when necessary)  
3) **Validate subgraphs**  
4) **Pre-merge validations** (fast checks before calling the merger)  
5) **Merge subgraphs** (delegate to the existing Rust merger)  
6) **Post-merge validations**  
7) **Optional satisfiability checks**

---

## What’s implemented

### Entry points
- **`compose_with_options(subgraphs, CompositionOptions)`**  
  Mirrors the JS `compose`, with a `run_satisfiability: bool` toggle.
- **`compose(subgraphs)`**  
  Convenience wrapper that uses `CompositionOptions::default()` (`run_satisfiability = true`).

### Lifecycle helpers
- **`expand_subgraphs(Vec<Subgraph<Initial>>) -> Result<Vec<Subgraph<Expanded>>, Vec<CompositionError>>`**
- **`upgrade_subgraphs_if_necessary(Vec<Subgraph<Expanded>>) -> Result<Vec<Subgraph<Upgraded>>, Vec<CompositionError>>`** (re-exported from schema upgrader)
- **`validate_subgraphs(Vec<Subgraph<Upgraded>>) -> Result<Vec<Subgraph<Validated>>, Vec<CompositionError>>`**
- **`pre_merge_validations(&[Subgraph<Validated>]) -> Result<(), Vec<CompositionError>>`**
- **`merge_subgraphs(Vec<Subgraph<Validated>>) -> Result<MergeOutput, Vec<CompositionError>>`**
  - Uses `merge_federation_subgraphs` and returns:
    ```rust
    pub struct MergeOutput {
        pub supergraph: Supergraph<Merged>,
        pub hints: Vec<String>,
    }
    ```
- **`post_merge_validations(&Supergraph<Merged>) -> Result<(), Vec<CompositionError>>`**
- **`validate_satisfiability(Supergraph<Merged>) -> Result<Supergraph<Satisfiable>, Vec<CompositionError>>`** (re-exported)

### Subgraph utility
- **`original_sdl()`** method wired across typestates so pre-merge logic can recover the authoring SDL where needed.
- **`schema_string()`** convenience retained for testing.

### SDL/AST-based validators (ported/built to mirror JS heuristics)
Pre-merge & post-merge phases use a mix of **AST-backed** and **SDL-heuristic** checks:

- **Directive definition compatibility**
  - **AST-backed** collection of `repeatable` and `locations` for each directive, aggregated across subgraphs.
  - Emits:
    - `CompositionError::DirectiveRepeatableConflict { directive }`
    - `CompositionError::DirectiveLocationsConflict { directive }`

- **Scalar `@specifiedBy(url: ...)` URL compatibility**
  - **AST-backed** collection of `specifiedBy` URLs per scalar across subgraphs.
  - Emits `CompositionError::ScalarSpecifiedByUrlConflict { scalar, urls }` when mismatched.

- **FieldSet sanity (`@key`, `@requires`, `@provides`)** (SDL heuristic)
  - `validate_directive_field_sets` and quick helpers:
    - **Syntax** via a small `parse_field_set_string` (flat paths only).
    - **Existence** for top-level fields on the same type.
  - Errors include `InvalidFieldSet`, `UnknownFieldInFieldSet`, and message-oriented fallbacks.
  - Note: Some invalid FieldSets may be caught *earlier* by GraphQL validation (e.g. `KeyInvalidFields` / “Cannot query field …”); tests accept both styles.

- **Join field type consistency** (SDL heuristic)  
  `validate_join_field_type_consistency_quick`: ensures `@join__field(graph:, type:)` is consistent per field across graphs.

- **No conflicting root types** (SDL heuristic, two passes)  
  - `validate_no_conflicting_root_types_quick`: normalized body comparison per type name.  
  - `validate_no_conflicting_root_types_enhanced`: compares per-field types per type occurrence.

- **`@key` referenced fields exist** (SDL heuristic)  
  `validate_key_fields_exist` checks type-local references.

### Hints
- We **collect** merge-time hints as `MergeOutput.hints: Vec<String>` from the merger.  
- We **do not yet** thread hints into the final `Supergraph<Satisfiable>` return value like the JS compose does. (See **Next steps**.)

---

## Design choices & rationale

- **Reuse the merger (`merge_federation_subgraphs`)**  
  This preserves correct federation semantics (join metadata, graph enum, extensions, collisions, error reporting) and avoids re-implementing deeply-tested logic in the JS stack.

- **AST-backed where robust, SDL heuristics where pragmatic**  
  Some quick checks from `compose.ts` are cheap and effective using SDL text. Where correctness mattered (e.g., directive compatibility, `@specifiedBy` URLs), AST collection is implemented to avoid brittle parsing.

- **Error aggregation**  
  Functions return `Result<_, Vec<CompositionError>>`, aggregating as many independent findings as possible (parity with JS’s “collect and report” philosophy).

- **Satisfiability toggle**  
  Matches JS behavior and allows faster composition in inner loops (e.g., editor flows).

- **`original_sdl` availability**  
  Several pre-merge checks are simpler using the authoring SDL (as in JS). We implemented `original_sdl()` across typestates to preserve access.

---

## Error handling & mapping

- **Pre-merge** returns `Vec<CompositionError>` containing our compatibility and FieldSet checks.
- **Merge**:
  - On success → `MergeOutput { supergraph, hints }`.
  - On failure → map merger errors to `CompositionError::TypeDefinitionInvalid` (or `InternalError` if no structured errors present).
- **Post-merge** re-validates using the merged supergraph SDL + AST to produce user-oriented messages.
- **GraphQL-level validation** may reject some inputs earlier than our SDL-based checks (e.g., malformed FieldSets). Tests are written to accept either error family (e.g., “invalid FieldSet”, “Cannot query field”, “KeyInvalidFields”).

> **Note:** Per your change log, `CompositionError` also includes a new `ValidationError` variant. Where available, we surface this instead of `InternalError` for clarity.

---

## Tests

Location: `apollo-federation/tests/composition_tests.rs`

### Snapshot-style SDL tests
- `can_compose_supergraph`
- `can_compose_with_descriptions`
- `can_compose_types_from_different_subgraphs`
- `compose_removes_federation_directives`

### Happy-path merge tests
- `merge_subgraphs_combines_types_and_fields_correctly`
- `compose_happy_path_basic`

### Negative/validation tests
- **Pre-merge conflicts**
  - Duplicate subgraph name
  - Conflicting directive `repeatable`
  - Conflicting directive `locations`
  - Scalar `@specifiedBy` URL mismatch
  - Invalid `@key` missing `fields:`
  - `@requires` referencing unknown field

- **Merge failures**
  - Conflicting field types across subgraphs

- **Post-merge checks**
  - `post_merge_validations_fail_on_invalid_key_directive` updated to accept both GraphQL validation (`KeyInvalidFields` / “Cannot query field …”) and SDL checks (“invalid FieldSet” / “does not exist”).

### Test helpers
- `mk_validated(name, url, sdl) -> TSubgraph<Validated>` to go from authoring SDL → `Validated` typestate (expand → upgrade → validate).
- `mk_two_validated(…) -> Vec<TSubgraph<Validated>>` convenience for merge tests.
- Where needed, `compose_with_options(..., run_satisfiability: false)` to bypass satisfiability for narrow assertions.

### Fixes applied while wiring tests
- Top-level `Subgraph` vs. typestate subgraph confusion resolved by using an alias like:
  ```rust
  use crate::subgraph::typestate::Subgraph as TSubgraph;
  ```
- Use `TSubgraph::<Initial>::parse(...)` when building inputs for the typestate pipeline.
- Exposed `to_string()` on merged supergraph’s inner schema for quick SDL assertions.
- `#[derive(Debug)]` added for `MergeOutput` to satisfy `unwrap_err()` bounds in tests that expect failures.

---

## How hints are handled (today vs. JS)

- **Today (Rust):** Merge-time hints are captured in `MergeOutput.hints` but **not** appended to the final `Supergraph<Satisfiable>`.
- **JS (compose.ts):** Appends merge and satisfiability hints to the final result.

### Suggested wiring (non-breaking)
1) Change `compose_with_options` to capture merge hints:
   ```rust
   let MergeOutput { supergraph: merged_supergraph, hints: merge_hints } = merge_subgraphs(validated_subgraphs)?;
   ```
2) When `run_satisfiability = true`, append satisfiability hints and return a `Supergraph<Satisfiable>` that stores a `Vec<CompositionHint>` (or `Vec<String>` translated via a simple converter).  
3) When `run_satisfiability = false`, consider returning a `Supergraph<Satisfiable>` with just merge hints (or an empty list) for parity. Document the behavior.

> For now, this implementation logs where hints are available and defers threading into the return type as a follow-up task.

---

## Implementation notes

- **`get_sdl_from_valid_fed_schema`** currently uses `to_string()` within `catch_unwind` as a fallback. Replace with a canonical SDL printer if/when exposed (e.g., `print_sdl`).  
- **FieldSet parser** intentionally minimal (no nested selections). If nested paths appear in your inputs, replace with a full parser or re-use an existing one from the federation crate.  
- **Mixed AST/SDL** is intentional to match the JS behavior quickly; we can migrate more checks to AST as needed.

---

## Limitations / Known gaps

- **Hints propagation** to the final `Supergraph<Satisfiable>` is not yet wired.
- **SDL heuristics** are not as robust as AST-based validation; we’ve already moved directive and specifiedBy checks to AST but left others as text-based.
- **Error type granularity** can be improved (separate “user” vs. “internal” failures).
- **Performance**: multiple string scans and SDL parsing heuristics can be optimized if needed.
- **Printer dependency**: replace `to_string()` fallback with a canonical schema printer to reduce flakiness.

---

## Next steps (recommended)

1) **Thread hints** into the final returned `Supergraph<Satisfiable>` to match JS parity.  
   - Option A: store as `Vec<String>`  
   - Option B: convert into `Vec<CompositionHint>` via a small adapter.

2) **Finish migrating pre/post-merge checks to AST** where feasible:  
   - Type-kind aggregation  
   - Field existence/ownership checks

3) **Adopt a canonical SDL printer** and delete the `catch_unwind` fallback.

4) **Strengthen FieldSet parsing** (or re-use a crate-internal parser) to support nesting, aliases, and other valid syntaxes.

5) **Expand tests**:  
   - Property tests for merge invariants  
   - More directive/location edge cases  
   - Round-trip tests (subgraph → merge → API schema)

6) **Doc comments & developer docs** for each validator: when it runs, what it guarantees, and example messages.

---

## API & compatibility notes

- `merge_subgraphs` now returns **`MergeOutput { supergraph, hints }`** rather than just a `Supergraph<Merged>`. Callers unaffected by hints can use `.supergraph` and ignore `.hints`.
- **Typestate** subgraph APIs are preserved; use `TSubgraph::<Initial>::parse` → expand → upgrade → validate for tests and pipelines.
- Errors raised at different stages are **intentionally accepted** by tests to avoid over-coupling behavior to any single stage.

---

## Appendix: Key symbols

- **Types:** `Initial`, `Expanded`, `Upgraded`, `Validated`, `Merged`, `Satisfiable`  
- **Core fns:** `compose_with_options`, `compose`, `expand_subgraphs`, `validate_subgraphs`, `pre_merge_validations`, `merge_subgraphs`, `post_merge_validations`, `validate_satisfiability`  
- **Utilities:** `original_sdl`, `schema_string`, `get_sdl_from_valid_fed_schema`, `extract_directive_arg_str`, `extract_directive_arg_token`, `parse_field_set_string`, `get_type_fields_map_from_sdl`

---

*Last updated:* (port status documenting AST-backed directive/`specifiedBy` checks, original SDL exposure, and test coverage for pre/merge/post stages.)
