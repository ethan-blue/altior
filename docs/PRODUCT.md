# Product contract

## Vision

Altior gives one person a durable AI identity and knowledge base that follows
them across their own devices while remaining useful offline. Models and agent
tools may change; the person's accumulated context must not.

## Primary user outcomes

1. Start a conversation with any supported agent harness.
2. Continue work after an app, process, or device restart.
3. Recall confirmed knowledge from another authorized device.
4. Inspect why Altior remembers something, correct it, or forget it everywhere.
5. Work against local projects with explicit file and terminal permissions.
6. Add skills, MCP servers, sync transports, and agent harnesses without changing
   the stable thread or memory model.

## Product vocabulary

- **Personal Vault**: the encrypted synchronization and ownership boundary.
- **Device**: one authorized installation with its own cryptographic identity.
- **Thread**: the Altior-owned conversation timeline, independent of an agent's
  provider-specific session identifier.
- **Turn**: one user request and the resulting agent event stream.
- **Harness**: an execution backend such as ACP, terminal, or Altior Native.
- **Memory**: a durable, scoped, explainable knowledge object.
- **Knowledge Document**: concurrently editable identity or project context.
- **Skill**: reusable instructions and resources discovered by Context Runtime.

## Non-goals for the first product

- organizations, teams, workspace membership, invitations, roles, or billing
- public sharing or collaboration between different people
- browser/mobile clients
- hosted model resale
- full source-code synchronization
- arbitrary in-process plugins
- an Altior-owned model/tool loop
- production multi-agent delegation or cross-harness thread migration

## First release surface

- Desktop: Chat, Threads, Projects, Memory, Agents, Settings
- Thread controls: harness, agent, model/mode when supported, permission profile,
  memory mode, project
- Memory controls: remember, forget, correct, inspect provenance, sync state
- Device controls: pair, rename, revoke, sync diagnostics, recovery export

The first production execution path is ACP. Other harnesses remain architectural
extension points until the ACP continuity journey passes acceptance.
