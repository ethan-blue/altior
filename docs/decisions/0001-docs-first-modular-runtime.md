# ADR 0001: Docs-first modular personal runtime

- Status: Accepted
- Date: 2026-08-29

## Context

The reference Lody codebase demonstrates valuable ACP lifecycle, local-first, and
failure-recovery behavior, but its product and implementation also include team,
cloud, authentication, and collaboration concerns that Altior explicitly rejects.
ACP and local-first frameworks continue to evolve.

## Decision

Build Altior as a new Rust domain/core with a Tauri client. Keep agent harness,
CRDT, sync transport, database, and UI implementations behind stable ports. Use
Lody and Zed as behavior references, not as Altior's package architecture.

## Consequences

- Initial delivery spends time on contracts and technical spikes.
- Altior avoids inheriting organization/cloud concepts and can survive ACP changes.
- Selected mature libraries reduce implementation load while remaining replaceable.
- Reusing source code requires explicit Apache-2.0 attribution and provenance.

## Revisit when

The P0 spikes show that a required boundary adds unacceptable latency, complexity,
or prevents a mature framework from providing its intended guarantees.

