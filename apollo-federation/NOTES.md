# Composition Port Notes

- Implemented composition pipeline in `src/composition/mod.rs`:
  - `pre_merge_validations(..)`: currently a no-op placeholder.
  - `merge_subgraphs(..)`: adapts validated subgraphs to the legacy merger in `src/merge.rs`, constructs `Supergraph<Merged>`.
  - `post_merge_validations(..)`: currently a no-op placeholder.
- Propagated merge hints into `Supergraph<Merged>` via a new constructor `Supergraph::<Merged>::new_with_hints(..)`.
- Added tests that exercise the end-to-end `composition::compose(..)` flow:
  - `tests/composition_pipeline.rs` includes:
    - happy path composition
    - error on conflicting non-shareable field sharing
    - error on satisfiability failure

How to run:

```bash
cargo test -p apollo-federation
```

Remarks:

- Satisfiability validation is always run, matching JS default behavior (`runSatisfiability` defaults to true). An options struct toggle can be added later if needed.
- Pre/post-merge validations beyond the merger's internal checks can be ported from JS in a follow-up.
