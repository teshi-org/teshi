# ACP Client architecture

ACP support is separate from Teshi's native LLM backend.

```text
                  Teshi Workflow
                        │
                  Agent Backend
                  /           \
                 /             \
        Native Backend      ACP Backend
             │                  │
         teshi-engine       teshi-acp
             │                  │
         LLM Provider       ACP stdio
                                │
                          External Agent
                                │
                             Cursor
```

## Native Agent vs ACP Agent

The native backend keeps using `teshi-engine` and its OpenAI/Anthropic-compatible LLM transport plus Teshi tool calls. ACP is not an LLM provider and is not implemented in `llm.rs`.

`teshi-acp` launches an external ACP agent such as Cursor CLI (`agent acp`). That external agent owns its reasoning and tool loop. Teshi acts as the ACP client: it initializes, negotiates capabilities, creates sessions, sends prompts, receives `session/update` events, answers permission requests, and shuts the process down.

## What lives where

- `teshi-acp`: JSON-RPC line framing, request correlation, stdio process launching, Cursor command defaults, permission policy, lifecycle state, ACP Registry parsing/fetching.
- `teshi-agent`: generic product concepts such as `ApprovalMode`, generation stages, and backend taxonomy (`Native` vs `Acp`).
- `teshi-engine`: native LLM transport remains unchanged.
- UI/frontends: choose a backend and render connection/auth/streaming/permission/error states without blocking their event loop.

## Lifecycle

```text
Created -> Starting -> Initialized -> Authenticated -> Ready -> ShuttingDown -> Exited
```

Invalid transitions return typed errors. Process exit fails pending requests so callers are not stranded.

## Permission flow

ACP `session/request_permission` is mapped to Teshi approval policy:

- Manual: forwarded to a host/UI callback.
- Auto: chooses the least-permissive supplied allow option, preferring allow-once.
- Bypass: chooses the broad supplied allow option, preferring allow-always.

Teshi never fabricates an option. If no safe allowing option exists, it rejects with a protocol error. These ACP permissions are independent from Teshi's human test-point review gate.

## Registry role

Teshi parses the official ACP Registry for discovery, metadata, platform resolution, and future managed installation. V1 does not install arbitrary Registry packages automatically. Cursor V1 prefers an explicitly configured executable, then a PATH lookup for `agent`, and launches it as structured arguments: `agent` + `acp`.

## Custom ACP configuration

Custom and Registry-installed agents share one user-level JSON file:

```text
<local-data-dir>/teshi/acp/installed.json
```

Run `teshi acp config-path` to print the exact absolute path on the current platform. Manage custom entries without editing JSON by hand:

```text
teshi acp list
teshi acp show codex --json
teshi acp add codex --program codex-acp --name "Codex" --arg acp
teshi acp add company-agent --program company-acp --arg --stdio --env API_KEY=COMPANY_ACP_KEY
teshi acp remove company-agent
```

`--arg` is repeatable and passes each value directly to the process; Teshi does not invoke a shell. `--env CHILD=SOURCE` records an environment-variable reference, and `--env NAME` is shorthand for `NAME=NAME`. Teshi reads the source variable only when it starts that agent. List, show, and the configuration file contain the source variable name, never its resolved value.

The configuration file is an array of definitions and may also be edited directly:

```json
[
  {
    "id": "company-agent",
    "name": "Company Agent",
    "program": "C:\\Tools\\company-acp.exe",
    "args": ["--stdio"],
    "env_from": {
      "API_KEY": "COMPANY_ACP_KEY"
    }
  }
]
```

IDs contain only ASCII letters, numbers, `-`, and `_`. IDs must be unique; names and programs must be non-empty. Invalid manual edits fail closed and are not partially loaded or rewritten. The legacy `env` object remains readable for Registry packages and existing records, but custom credentials should use `env_from` so secret values stay outside this file.

Adding an existing ID replaces that definition. Configuration does not select an agent globally: use `/agent` in the TUI to select one for the current conversation.

## ACP vs MCP

ACP is Agent-to-Client integration: Teshi connects to an external agent. MCP is Agent-to-Tool integration: an agent connects to external tools. They are intentionally not conflated.
