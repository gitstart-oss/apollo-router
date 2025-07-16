
##  Task Summary

I Implemented the `merge_repeatable_directives` function in `apollo-federation/src/merge.rs`. This function merges two sets of GraphQL `DirectiveDefinition`s, to handle both repeatable and non-repeatable directives according to the the accessment note specification.

---

##  Here's What I Did

- I Implemented the logic to:
  - **Override** non-repeatable directives in `existing` with those from `new_defs`.
  - **Accumulate** repeatable directives from both `existing` and `new_defs`.
- Then, I went on to write series of **unit tests** to verify:
  - the correct behavior for repeatable vs. non-repeatable directives.
  - Edge cases like merging empty lists.
  - Preservation of descriptions, names, and other fields.

---

## Here's are the code Behavior Details

```rust
fn merge_repeatable_directives(
    existing: &[DirectiveDefinition],
    new_defs: &[DirectiveDefinition],
) -> Vec<DirectiveDefinition> {
    let mut result = existing.to_vec();

    for new_dir in new_defs {
        if let Some(pos) = result.iter().position(|d| d.name == new_dir.name) {
            if !new_dir.repeatable {
                result[pos] = new_dir.clone(); // this overrides existing definition
            } else {
                result.push(new_dir.clone()); // this accumulates
            }
        } else {
            result.push(new_dir.clone()); // this is for new directive
        }
    }

    result
}
```

# Test Coverage

Unit tests were added directly in `merge.rs` under the `#[cfg(test)]` module to ensure correct behavior of the `merge_repeatable_directives` function.

## Here are Included Tests

- **`test_merge_repeatable_directives`**  
   This Merges a mix of repeatable and non-repeatable directives.

- **`test_merge_empty_lists`**  
   This Handles merging when both inputs are empty.

- **`test_merge_with_empty_existing`**  
   This Merges when only new directives are present.

- **`test_merge_with_empty_new`**  
   This Merges when only existing directives are present.

- **`test_multiple_repeatable_directives`**  
   This Validates accumulation of multiple instances of a repeatable directive with the same name.

---


To run the test suite and capture detailed output, use:

```bash
cargo test merge::tests -- --nocapture
```
If you have any issues, feel free to reach out to me via email: [ndemamanuel2002@gmail.com](mailto:ndemamanuel2002@gmail.com).



