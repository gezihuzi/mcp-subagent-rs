# Rust Service Refactor Plan

## Objective

Refactor invoice calculation flow for readability and safety without changing expected business output for valid requests.

## Scope

In scope: `src/lib.rs` pricing helpers, discount validation, unit tests for edge inputs.
Out of scope: API transport layer, database schema, authentication.

## Stages

1. Research: inspect existing helper behavior and identify edge-case risk.
2. Plan: confirm refactor steps and expected acceptance checks.
3. Build: implement incremental refactor + tests.
4. Review: run correctness review before merge.
5. Archive: capture decisions and residual risks.

## Steps

1. [ ] Introduce explicit input validation for amount and discount rate.
2. [ ] Split pricing math into focused helpers with clear names.
3. [ ] Add regression tests for boundary values (`0`, `100`, invalid discount).

## Validation

- `cargo test -q`
- No behavior regression for existing valid examples.
