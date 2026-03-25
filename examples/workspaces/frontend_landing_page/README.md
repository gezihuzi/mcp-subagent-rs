# Frontend Landing Page Example

This example demonstrates a UI-focused workflow where layout and styling changes are implemented by a frontend agent and then reviewed before archive.

## Suggested Delegation

- `fast-researcher`: gather UX/a11y risks and summarize baseline issues.
- `frontend-builder`: implement markup/style updates from `PLAN.md`.
- `style-reviewer`: review readability, consistency, and UX copy clarity.

## Example Task Prompt

Use mcp-subagent to improve this landing page workspace.
Update PLAN.md first, then run frontend-builder in Build stage for `web/index.html` and `web/styles.css`.
After implementation, run style-reviewer in Review stage and report unresolved UI risks.

## Workspace Files

- `PROJECT.md`: persistent product constraints and quality goals.
- `PLAN.md`: active implementation plan.
- `web/index.html`: page structure and copy.
- `web/styles.css`: visual language and responsive layout.
