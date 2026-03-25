# Rust Service Refactor Project Memory

## Product Context

This workspace simulates a backend billing service used by internal APIs. The current implementation computes invoice totals and applies discounts, but the code has low readability and weak guardrails around invalid input.

## Refactor Goals

- Keep behavior stable for valid inputs.
- Reject invalid discount rates and negative usage values.
- Improve test coverage around invoice edge cases.

## Collaboration Contract

- Plan first, then implement in small diffs.
- Build changes go through `backend-coder`.
- Correctness checks go through `correctness-reviewer` before archive.
