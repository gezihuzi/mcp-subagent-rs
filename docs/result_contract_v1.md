# Result Contract v1 (`mcp-subagent.result.v1`)

Status: active  
Version: `mcp-subagent.result.v1`  
Applies to:

- CLI: `mcp-subagent result <handle-id> --json`
- MCP tool: `get_run_result`

This document defines the machine-readable result contract for host integrations.

## 1. Compatibility Policy

- `contract_version` is required and must equal `mcp-subagent.result.v1`.
- New fields may be added in future versions.
- Existing `v1` fields will not change meaning.
- Consumers should ignore unknown fields.
- `normalized_result` is optional and may be `null`.

## 2. Shared Top-Level Fields

Both CLI and MCP responses include:

- `contract_version: string`
- `handle_id: string`
- `status: string` (`running|succeeded|failed|timed_out|cancelled|...`)
- `updated_at: string` (MCP only, RFC3339)
- `normalization_status: string` (`Validated|Degraded|Invalid|NotAvailable`)
- `summary: string|null`
- `native_result: string|null`
- `normalized_result: object|null`
- `provider_exit_code: number|null`
- `retries: number`
- `usage: object`
- `error_message: string|null`
- `artifact_index: array`

## 3. CLI vs MCP Differences

`result --json` (CLI) includes:

- `view: string` (`raw|normalized|summary|auto`)
- `normalized_result` uses `SummaryEnvelope` shape:
  - `contract_version`
  - `parse_status`
  - `summary` (nested structured summary)
  - `raw_fallback_text`

`get_run_result` (MCP) includes:

- `updated_at: string` (RFC3339)
- `provider: string|null`
- `model: string|null`
- `normalized_result` uses flattened `SummaryOutput` shape:
  - `contract_version`
  - `parse_status`
  - `summary`
  - `key_findings`
  - `open_questions`
  - `next_steps`
  - `exit_code`
  - `verification_status`
  - `touched_files`
  - `plan_refs`

## 4. Usage Object (`usage`)

`usage` fields:

- `started_at: string|null`
- `finished_at: string|null`
- `duration_ms: number|null`
- `provider: string`
- `model: string|null`
- `provider_exit_code: number|null`
- `retries: number`
- `token_source: string` (`native|estimated|mixed|unknown`)
- `input_tokens: number|null`
- `output_tokens: number|null`
- `total_tokens: number|null`
- `estimated_prompt_bytes: number|null`
- `estimated_output_bytes: number|null`

## 5. Minimal Examples

CLI (`result --json`):

```json
{
  "contract_version": "mcp-subagent.result.v1",
  "handle_id": "019dxxxx",
  "status": "succeeded",
  "view": "summary",
  "normalization_status": "Validated",
  "summary": "Task completed.",
  "native_result": "...",
  "normalized_result": null,
  "provider_exit_code": 0,
  "retries": 0,
  "usage": {
    "token_source": "estimated"
  },
  "error_message": null,
  "artifact_index": []
}
```

MCP (`get_run_result`):

```json
{
  "contract_version": "mcp-subagent.result.v1",
  "handle_id": "019dxxxx",
  "status": "succeeded",
  "updated_at": "2026-03-25T08:00:00Z",
  "provider": "Codex",
  "model": "gpt-5.3-codex",
  "normalization_status": "Validated",
  "summary": "Task completed.",
  "native_result": "...",
  "normalized_result": null,
  "provider_exit_code": 0,
  "retries": 0,
  "usage": {
    "token_source": "estimated"
  },
  "error_message": null,
  "artifact_index": []
}
```
