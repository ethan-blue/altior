# Reference architecture notes

These projects are behavior references, not package blueprints. Altior adopts a
pattern only when it preserves the personal, local-first, offline, and encrypted
product invariants in `AGENTS.md`.

## Lody

Study:

- ACP startup, authentication, capability normalization, steering, cancellation,
  and session recovery
- supervised subprocess lifecycle and managed runtime updates
- durable operation records, idempotent materialization, and crash reconciliation
- worktree isolation for parallel agent work
- synthetic protocol transcripts and executable race models

Do not import:

- workspace membership, remote-machine sharing, hosted access checks, or cloud
  product composition
- Loro documents as an automatic answer for every durable record
- provider or ACP records as Altior domain records

## Cumora

Study:

- persistent local engine sessions and deployment-independent runtime clients
- wake debounce/coalescing and durable inbox catch-up
- seen-boundary checks that prevent stale or duplicate output
- single-use acknowledgement tokens for explicit stale-state overrides
- pure streaming reducers, idle/wall timeouts, and production-failure fixtures

Do not import:

- companies, participants, team chat, billing, managed cloud pods, or server-side
  plaintext knowledge
- Postgres/Redis as the Personal Vault source of truth

## Alma

Study:

- one local desktop surface across model providers
- memory-first context, explicit memory management, and conversation archives
- discoverable skills, tools, MCP integrations, and project-local extensions
- durable tasks/goals and references between threads, files, agents, skills, and
  other objects
- channels and automation as optional extensions around the same personal agent

Do not import:

- prompt-defined identity as a security boundary
- untyped configuration trees for durable protocol or synchronization state
- credentials in ordinary configuration files or provider objects
- a monolithic desktop process that owns data, UI, and agent execution together

## Codex

Study:

- the open harness as a reusable execution layer rather than an application
  domain model
- app-server threads, turns, streamed events, approvals, and bidirectional RPC
- local stdio integration before experimental remote transports
- sandbox and approval policy owned by the host application
- visible child-agent threads with summaries returned to a parent thread
- generated protocol schemas and forward-compatible notification handling

Do not import:

- coding-specific thread items into Altior's stable domain schema
- Codex authentication, provider sessions, or compaction as Personal Vault facts
- implicit parallel writes without worktree or resource isolation

Official OpenAI references:

- [Codex App Server](https://developers.openai.com/codex/app-server)
- [Codex open-source components](https://developers.openai.com/codex/open-source)
- [Codex as a platform](https://developers.openai.com/blog/codex-as-a-platform)
- [Codex subagents](https://developers.openai.com/codex/agent-configuration/subagents)

## Altior synthesis

Altior combines the useful seams, not the surrounding products:

```text
Personal identity, memory, skills, references, schedules
                         |
               Context Runtime and policy
                         |
       Aggregated Agent Runtime and operation journal
              /          |          |          \
            ACP      Codex AS    Terminal     Native
              \          |          |          /
               normalized events and artifacts
                         |
          local projections + encrypted Personal Vault
```

The result is one personal agent system with several interchangeable execution
engines. The engine can change; the person's identity, knowledge, provenance,
permissions, and task history remain Altior-owned.
