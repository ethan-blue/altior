# Third-Party Notices

Altior is licensed under the Apache License, Version 2.0.

This file documents third-party open-source software packages incorporated into
or referenced by Altior, along with their applicable licenses and provenance.

## External Dependencies

### Rust Ecosystem (Crates)

1. **rusqlite**
   - License: MIT License
   - Copyright (c) 2014-2024 The Rusqlite Developers
   - Role: SQLite C-API wrapper used for local projection indexing and memory retrieval.

2. **serde & serde_json**
   - License: MIT License / Apache License, Version 2.0
   - Copyright (c) 2014-2024 Erick Tryzelaar and David Tolnay
   - Role: Data serialization and deserialization across IPC and protocol envelopes.

3. **tokio**
   - License: MIT License
   - Copyright (c) 2024 Tokio Contributors
   - Role: Async runtime for local OS IPC (Named Pipes, Unix Domain Sockets) and subprocess supervision.

4. **RustCrypto Primitives (sha2, chacha20poly1305, hkdf)**
   - License: MIT License / Apache License, Version 2.0
   - Copyright (c) 2018-2024 RustCrypto Developers
   - Role: Authenticated symmetric encryption and deterministic key derivation.

5. **x25519-dalek**
   - License: BSD-3-Clause License
   - Copyright (c) 2016-2021 Isis Lovecruft, Henry de Valence
   - Role: Elliptic curve Diffie-Hellman key exchange for device pairing spikes.

6. **automerge**
   - License: MIT License
   - Copyright (c) 2017-2024 Automerge Contributors
   - Role: CRDT document synchronization engine evaluated behind SyncDocumentEngine.

7. **Tauri v2 (tauri, tauri-build)**
   - License: MIT License / Apache License, Version 2.0
   - Copyright (c) 2019-2024 Tauri Programme within The Commons Conservancy
   - Role: Thin desktop windowing shell (kept in decoupled crate apps/desktop/src-tauri).

### Node.js & Web Ecosystem (Desktop Client)

1. **React & React DOM**
   - License: MIT License
   - Copyright (c) Meta Platforms, Inc. and affiliates
   - Role: UI component rendering for Altior Desktop.

2. **Vite & @vitejs/plugin-react**
   - License: MIT License
   - Copyright (c) 2019-present Evan You & Vite Contributors
   - Role: Frontend build tool and local dev server.

3. **Playwright**
   - License: Apache License, Version 2.0
   - Copyright (c) Microsoft Corporation
   - Role: Browser automation for visual baseline and geometry regression gates.

4. **Vitest**
   - License: MIT License
   - Copyright (c) 2021-present Anthony Fu and Vitest contributors
   - Role: Unit and component test runner.

## Provenance and Code Reuse Statement

- **No source dependency on Lody**: Per Altior contributor rules (AGENTS.md), there is no source dependency on or uncredited copy from Lody or any other external proprietary or open-source codebase.
- All code within the `crates/` and `apps/` trees has been cleanly designed and implemented to satisfy the architectural contracts defined under `docs/`.
