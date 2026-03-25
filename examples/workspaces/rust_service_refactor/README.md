# Rust Service Refactor Example

This example demonstrates a backend-oriented workflow where a supervisor delegates build and correctness review to separate subagents.

## Suggested Delegation

- `fast-researcher`: map risky modules and summarize edge-case behavior.
- `backend-coder`: implement scoped Rust refactor from `PLAN.md`.
- `correctness-reviewer`: verify behavior and tests before archive.

## Example Task Prompt

Use mcp-subagent to refactor invoice calculation in this workspace.
First update PLAN.md with concrete steps, then run backend-coder in Build stage.
After build completion, run correctness-reviewer in Review stage and report any regressions.

## Workspace Files

- `PROJECT.md`: persistent project memory and constraints.
- `PLAN.md`: active implementation plan.
- `src/lib.rs`: sample backend logic with testable behavior.
