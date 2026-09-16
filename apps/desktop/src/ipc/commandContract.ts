/**
 * TypeScript mirror of the Rust command-envelope contract (A01, ADR 0004/0019).
 *
 * Real Core validates every incoming command at the serde boundary: typed
 * identifier newtypes (`thr_`, `trn_`, `agp_`, `hsb_`, `evt_`, `op_` bodies
 * of 16..64 chars from `[0-9a-z]`), bounded text fields, closed enums, and
 * payload invariants (e.g. env_keys.length === secret_refs.length). The
 * in-memory fixture transport runs the same checks here so a command the
 * production Core would reject cannot pass silently in development or tests.
 *
 * This module mirrors the checks that guard command *validity*; it does not
 * replicate Core business rules (existence, capability, delivery state).
 */

import type { CommandEnvelope } from "./dto/CommandEnvelope";

const textEncoder = new TextEncoder();

function byteLength(value: string): number {
  return textEncoder.encode(value).length;
}

const DOMAIN_BODY = /^[0-9a-z]{16,64}$/;

function isValidDomainId(value: unknown, prefix: string): boolean {
  return (
    typeof value === "string" &&
    value.startsWith(`${prefix}_`) &&
    DOMAIN_BODY.test(value.slice(prefix.length + 1))
  );
}

function isValidOptionalId(value: unknown, prefix: string): boolean {
  return value === null || value === undefined || isValidDomainId(value, prefix);
}

function isBoundedText(value: unknown, maxBytes: number): boolean {
  return typeof value === "string" && byteLength(value) <= maxBytes;
}

const HARNESS_KINDS = new Set(["acp", "terminal", "native"]);
const MEMORY_MODES = new Set(["off", "session", "long_term"]);
const DECISIONS = new Set(["approved", "denied"]);
const IDENTITY_KINDS = new Set(["name", "about", "preference", "instruction"]);
const MEMORY_SCOPE_KINDS = new Set(["global", "person", "project", "thread"]);
const MEMORY_KINDS = new Set(["fact", "preference", "instruction", "summary"]);
const MEMORY_STATES = new Set(["candidate", "confirmed", "rejected", "superseded", "forgotten", "expired"]);
const MEMORY_SENSITIVITIES = new Set(["normal", "sensitive"]);
const MEMORY_SOURCES = new Set(["explicit", "inferred"]);

const LIMITS = {
  payloadBytes: 64 * 1024,
  messageText: 64 * 1024,
  threadTitle: 512,
  searchQuery: 256,
  displayName: 256,
  path: 4096,
  identityContent: 4096,
  secretRef: 256,
  threadList: 200,
  historyLimit: 500,
  identityDocList: 32,
  contextSnapshotList: 50,
  memoryContent: 8192,
  memoryExcerpt: 1024,
  memoryReason: 512,
  memoryList: 100,
  harnessArgs: 256,
  harnessArgText: 64 * 1024,
  harnessEnvKeys: 256,
  harnessEnvKeyText: 256,
  harnessSecretRefs: 256,
};

function isValidHarnessEnvKey(value: unknown): boolean {
  if (typeof value !== "string" || value.length === 0 || byteLength(value) > LIMITS.harnessEnvKeyText) {
    return false;
  }
  const first: string = value[0] ?? "";
  if (!(/[A-Za-z_]/.test(first))) return false;
  return /^[A-Za-z0-9_]+$/.test(value);
}

function isValidSecretRef(value: unknown): boolean {
  return (
    typeof value === "string" &&
    value.trim().length > 0 &&
    byteLength(value) <= LIMITS.secretRef &&
    !/[\u0000-\u001f\u007f]/.test(value)
  );
}

interface BindingShape {
  harness_binding_id?: unknown;
  agent_profile_id?: unknown;
  program: unknown;
  args?: unknown;
  env_keys?: unknown;
  secret_refs?: unknown;
  label?: unknown;
}

function asBindingShape(value: Record<string, unknown>): BindingShape {
  return value as unknown as BindingShape;
}

function validateBinding(binding: BindingShape): string | null {
  if (typeof binding.program !== "string" || binding.program.trim().length === 0) {
    return "binding.program must be a non-empty string";
  }
  if (byteLength(binding.program) > LIMITS.path) {
    return "binding.program exceeds the path byte limit";
  }
  if (
    binding.label !== undefined &&
    binding.label !== null &&
    (!isBoundedText(binding.label, LIMITS.displayName) || (binding.label as string).trim().length === 0)
  ) {
    return "binding.label must be a bounded non-empty string";
  }
  const args = binding.args ?? [];
  if (!Array.isArray(args) || args.length > LIMITS.harnessArgs) {
    return "binding.args exceeds the count limit";
  }
  if (!args.every((a) => isBoundedText(a, LIMITS.harnessArgText))) {
    return "binding.args contains an oversized argument";
  }
  const envKeys = binding.env_keys ?? [];
  if (!Array.isArray(envKeys) || envKeys.length > LIMITS.harnessEnvKeys) {
    return "binding.env_keys exceeds the count limit";
  }
  if (!envKeys.every(isValidHarnessEnvKey)) {
    return "binding.env_keys contains an invalid key";
  }
  const secretRefs = binding.secret_refs ?? [];
  if (!Array.isArray(secretRefs) || secretRefs.length > LIMITS.harnessSecretRefs) {
    return "binding.secret_refs exceeds the count limit";
  }
  if (!secretRefs.every(isValidSecretRef)) {
    return "binding.secret_refs contains an invalid reference";
  }
  if (envKeys.length !== secretRefs.length) {
    return "binding env_keys and secret_refs counts must match";
  }
  if (!isValidOptionalId(binding.harness_binding_id, "hsb")) {
    return "binding.harness_binding_id is not a valid hsb_ identifier";
  }
  if (!isValidOptionalId(binding.agent_profile_id, "agp")) {
    return "binding.agent_profile_id is not a valid agp_ identifier";
  }
  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function validatePayload(kind: string, payload: unknown): string | null {
  // Payload-free commands: the envelope may carry a null payload.
  if (kind === "ping" || kind === "cancel" || kind === "subscribe" || kind === "request_snapshot") {
    return null;
  }
  if (!isRecord(payload)) {
    return `${kind} requires a payload object`;
  }
  switch (kind) {
    case "list_threads": {
      if (payload.limit !== null && payload.limit !== undefined && (typeof payload.limit !== "number" || payload.limit > LIMITS.threadList)) {
        return "list_threads limit is out of bounds";
      }
      return null;
    }
    case "search_threads": {
      if (!isBoundedText(payload.query, LIMITS.searchQuery)) {
        return "search_threads query exceeds the byte limit";
      }
      if (payload.limit !== null && payload.limit !== undefined && (typeof payload.limit !== "number" || payload.limit > LIMITS.threadList)) {
        return "search_threads limit is out of bounds";
      }
      return null;
    }
    case "create_thread": {
      if (!isValidDomainId(payload.agent_profile_id, "agp")) {
        return "create_thread agent_profile_id is not a valid agp_ identifier";
      }
      if (payload.title !== null && payload.title !== undefined && !isBoundedText(payload.title, LIMITS.threadTitle)) {
        return "create_thread title exceeds the byte limit";
      }
      if (!isValidOptionalId(payload.project_id, "prj")) {
        return "create_thread project_id is not a valid prj_ identifier";
      }
      return null;
    }
    case "open_thread": {
      if (!isValidDomainId(payload.thread_id, "thr")) {
        return "open_thread thread_id is not a valid thr_ identifier";
      }
      if (payload.history_limit !== null && payload.history_limit !== undefined && (typeof payload.history_limit !== "number" || payload.history_limit > LIMITS.historyLimit)) {
        return "open_thread history_limit is out of bounds";
      }
      return null;
    }
    case "get_history": {
      if (!isValidDomainId(payload.thread_id, "thr")) {
        return "get_history thread_id is not a valid thr_ identifier";
      }
      if (payload.limit !== null && payload.limit !== undefined && (typeof payload.limit !== "number" || payload.limit > LIMITS.historyLimit)) {
        return "get_history limit is out of bounds";
      }
      return null;
    }
    case "configure_agent": {
      if (!isValidOptionalId(payload.agent_profile_id, "agp")) {
        return "configure_agent agent_profile_id is not a valid agp_ identifier";
      }
      if (!isBoundedText(payload.display_name, LIMITS.displayName) || (payload.display_name as string).trim().length === 0) {
        return "configure_agent display_name must be a bounded non-empty string";
      }
      if (typeof payload.preferred_harness !== "string" || !HARNESS_KINDS.has(payload.preferred_harness)) {
        return "configure_agent preferred_harness must be acp, terminal, or native";
      }
      if (typeof payload.memory_mode !== "string" || !MEMORY_MODES.has(payload.memory_mode)) {
        return "configure_agent memory_mode must be off, session, or long_term";
      }
      if (payload.binding !== null && payload.binding !== undefined) {
        if (!isRecord(payload.binding)) return "configure_agent binding must be an object";
        const bindingError = validateBinding(asBindingShape(payload.binding));
        if (bindingError) return bindingError;
      }
      return null;
    }
    case "test_harness_binding": {
      if (payload.harness_binding_id !== null && payload.harness_binding_id !== undefined) {
        if (!isValidDomainId(payload.harness_binding_id, "hsb")) {
          return "test_harness_binding harness_binding_id is not a valid hsb_ identifier";
        }
      }
      if (typeof payload.program !== "string" || payload.program.trim().length === 0) {
        return "test_harness_binding program must be a non-empty string";
      }
      if (byteLength(payload.program) > LIMITS.path) {
        return "test_harness_binding program exceeds the path byte limit";
      }
      const bindingError = validateBinding(asBindingShape(payload));
      if (bindingError) return bindingError;
      return null;
    }
    case "start_turn": {
      if (!isValidDomainId(payload.thread_id, "thr")) {
        return "start_turn thread_id is not a valid thr_ identifier";
      }
      if (!isValidOptionalId(payload.turn_id, "trn")) {
        return "start_turn turn_id is not a valid trn_ identifier (or null for Core allocation)";
      }
      if (!isBoundedText(payload.prompt, LIMITS.messageText)) {
        return "start_turn prompt exceeds the byte limit";
      }
      return null;
    }
    case "cancel_turn": {
      if (!isValidDomainId(payload.thread_id, "thr")) {
        return "cancel_turn thread_id is not a valid thr_ identifier";
      }
      if (!isValidOptionalId(payload.turn_id, "trn")) {
        return "cancel_turn turn_id is not a valid trn_ identifier";
      }
      if (!isValidOptionalId(payload.target_operation_id, "op")) {
        return "cancel_turn target_operation_id is not a valid op_ identifier";
      }
      return null;
    }
    case "respond_permission": {
      if (!isValidDomainId(payload.event_id, "evt")) {
        return "respond_permission event_id is not a valid evt_ identifier";
      }
      if (typeof payload.decision !== "string" || !DECISIONS.has(payload.decision)) {
        return "respond_permission decision must be approved or denied";
      }
      return null;
    }
    case "runtime_status": {
      if (typeof payload.include_diagnostics !== "boolean") {
        return "runtime_status include_diagnostics must be a boolean";
      }
      return null;
    }
    case "diagnostics": {
      if (!isValidOptionalId(payload.thread_id, "thr")) {
        return "diagnostics thread_id is not a valid thr_ identifier";
      }
      return null;
    }
    case "put_identity_document": {
      if (!isValidOptionalId(payload.document_id, "idd")) {
        return "put_identity_document document_id is not a valid idd_ identifier";
      }
      if (typeof payload.kind !== "string" || !IDENTITY_KINDS.has(payload.kind)) {
        return "put_identity_document kind must be name, about, preference, or instruction";
      }
      if (!isBoundedText(payload.content, LIMITS.identityContent) || (payload.content as string).trim().length === 0) {
        return "put_identity_document content must be a bounded non-empty string";
      }
      return null;
    }
    case "delete_identity_document": {
      if (!isValidDomainId(payload.document_id, "idd")) {
        return "delete_identity_document document_id is not a valid idd_ identifier";
      }
      return null;
    }
    case "list_identity_documents": {
      if (payload.limit !== null && payload.limit !== undefined) {
        const limit = payload.limit;
        if (typeof limit !== "number" || limit < 1 || limit > LIMITS.identityDocList) {
          return "list_identity_documents limit is out of bounds";
        }
      }
      return null;
    }
    case "get_context_snapshot": {
      const hasTurn = payload.turn_id !== null && payload.turn_id !== undefined;
      const hasThread = payload.thread_id !== null && payload.thread_id !== undefined;
      if (!hasTurn && !hasThread) {
        return "get_context_snapshot requires turn_id or thread_id";
      }
      if (hasTurn && !isValidDomainId(payload.turn_id, "trn")) {
        return "get_context_snapshot turn_id is not a valid trn_ identifier";
      }
      if (hasThread && !isValidDomainId(payload.thread_id, "thr")) {
        return "get_context_snapshot thread_id is not a valid thr_ identifier";
      }
      if (payload.limit !== null && payload.limit !== undefined) {
        const limit = payload.limit;
        if (typeof limit !== "number" || limit < 1 || limit > LIMITS.contextSnapshotList) {
          return "get_context_snapshot limit is out of bounds";
        }
      }
      return null;
    }
    case "list_memories": {
      if (payload.scope_kind !== null && payload.scope_kind !== undefined) {
        if (typeof payload.scope_kind !== "string" || !MEMORY_SCOPE_KINDS.has(payload.scope_kind)) {
          return "list_memories scope_kind must be global, person, project, or thread";
        }
        if (payload.scope_kind === "global") {
          if (payload.scope_target !== null && payload.scope_target !== undefined && payload.scope_target !== "") {
            return "list_memories global scope must not specify scope_target";
          }
        } else {
          if (typeof payload.scope_target !== "string" || payload.scope_target.trim().length === 0) {
            return "list_memories scoped query requires non-empty scope_target";
          }
        }
      }
      if (payload.state !== null && payload.state !== undefined) {
        if (typeof payload.state !== "string" || !MEMORY_STATES.has(payload.state)) {
          return "list_memories state is invalid";
        }
      }
      if (payload.limit !== null && payload.limit !== undefined) {
        const limit = payload.limit;
        if (typeof limit !== "number" || !Number.isInteger(limit) || limit < 1 || limit > LIMITS.memoryList) {
          return "list_memories limit is out of bounds (1..=100)";
        }
      }
      if (payload.cursor !== null && payload.cursor !== undefined) {
        if (!isRecord(payload.cursor)) {
          return "list_memories cursor must be an object";
        }
        if (typeof payload.cursor.updated_at !== "number" || !Number.isFinite(payload.cursor.updated_at)) {
          return "list_memories cursor updated_at must be a finite number";
        }
        if (!isValidDomainId(payload.cursor.memory_id, "mem")) {
          return "list_memories cursor memory_id is not a valid mem_ identifier";
        }
      }
      return null;
    }
    case "propose_memory": {
      if (!isBoundedText(payload.content, LIMITS.memoryContent) || (payload.content as string).trim().length === 0) {
        return "propose_memory content must be a bounded non-empty string";
      }
      if (typeof payload.scope_kind !== "string" || !MEMORY_SCOPE_KINDS.has(payload.scope_kind)) {
        return "propose_memory scope_kind must be global, person, project, or thread";
      }
      if (payload.scope_kind === "global") {
        if (payload.scope_target !== null && payload.scope_target !== undefined && payload.scope_target !== "") {
          return "propose_memory global scope must not specify scope_target";
        }
      } else {
        if (typeof payload.scope_target !== "string" || payload.scope_target.trim().length === 0) {
          return "propose_memory scoped memory requires non-empty scope_target";
        }
      }
      if (typeof payload.kind !== "string" || !MEMORY_KINDS.has(payload.kind)) {
        return "propose_memory kind must be fact, preference, instruction, or summary";
      }
      if (payload.sensitivity !== null && payload.sensitivity !== undefined) {
        if (typeof payload.sensitivity !== "string" || !MEMORY_SENSITIVITIES.has(payload.sensitivity)) {
          return "propose_memory sensitivity must be normal or sensitive";
        }
      }
      if (payload.confidence !== null && payload.confidence !== undefined) {
        const conf = payload.confidence;
        if (typeof conf !== "number" || !Number.isInteger(conf) || conf < 0 || conf > 100) {
          return "propose_memory confidence must be an integer between 0 and 100";
        }
      }
      if (payload.source !== null && payload.source !== undefined) {
        if (typeof payload.source !== "string" || !MEMORY_SOURCES.has(payload.source)) {
          return "propose_memory source must be explicit or inferred";
        }
      }
      if (!isValidOptionalId(payload.thread_id, "thr")) {
        return "propose_memory thread_id is not a valid thr_ identifier";
      }
      if (!isValidOptionalId(payload.turn_id, "trn")) {
        return "propose_memory turn_id is not a valid trn_ identifier";
      }
      if (payload.excerpt !== null && payload.excerpt !== undefined) {
        if (!isBoundedText(payload.excerpt, LIMITS.memoryExcerpt)) {
          return "propose_memory excerpt exceeds the byte limit";
        }
      }
      return null;
    }
    case "confirm_memory": {
      if (!isValidDomainId(payload.memory_id, "mem")) {
        return "confirm_memory memory_id is not a valid mem_ identifier";
      }
      return null;
    }
    case "reject_memory": {
      if (!isValidDomainId(payload.memory_id, "mem")) {
        return "reject_memory memory_id is not a valid mem_ identifier";
      }
      if (payload.reason !== null && payload.reason !== undefined) {
        if (!isBoundedText(payload.reason, LIMITS.memoryReason)) {
          return "reject_memory reason exceeds the byte limit";
        }
      }
      return null;
    }
    case "correct_memory": {
      if (!isValidDomainId(payload.memory_id, "mem")) {
        return "correct_memory memory_id is not a valid mem_ identifier";
      }
      if (!isBoundedText(payload.content, LIMITS.memoryContent) || (payload.content as string).trim().length === 0) {
        return "correct_memory content must be a bounded non-empty string";
      }
      if (payload.scope_kind !== null && payload.scope_kind !== undefined) {
        if (typeof payload.scope_kind !== "string" || !MEMORY_SCOPE_KINDS.has(payload.scope_kind)) {
          return "correct_memory scope_kind must be global, person, project, or thread";
        }
        if (payload.scope_kind === "global") {
          if (payload.scope_target !== null && payload.scope_target !== undefined && payload.scope_target !== "") {
            return "correct_memory global scope must not specify scope_target";
          }
        } else {
          if (typeof payload.scope_target !== "string" || payload.scope_target.trim().length === 0) {
            return "correct_memory scoped memory requires non-empty scope_target";
          }
        }
      }
      if (payload.kind !== null && payload.kind !== undefined) {
        if (typeof payload.kind !== "string" || !MEMORY_KINDS.has(payload.kind)) {
          return "correct_memory kind must be fact, preference, instruction, or summary";
        }
      }
      if (payload.sensitivity !== null && payload.sensitivity !== undefined) {
        if (typeof payload.sensitivity !== "string" || !MEMORY_SENSITIVITIES.has(payload.sensitivity)) {
          return "correct_memory sensitivity must be normal or sensitive";
        }
      }
      return null;
    }
    case "forget_memory": {
      if (!isValidDomainId(payload.memory_id, "mem")) {
        return "forget_memory memory_id is not a valid mem_ identifier";
      }
      return null;
    }
    default:
      return `unknown command kind: ${kind}`;
  }
}

/**
 * Returns a human-readable rejection reason, or null when the envelope is
 * contract-valid. Mirrors real Core's serde + payload validation order:
 * envelope identity fields first, then the typed payload.
 */
export function validateCommandEnvelope(envelope: CommandEnvelope): string | null {
  if (envelope.protocol_version !== 1) {
    return `unsupported protocol version: ${String(envelope.protocol_version)}`;
  }
  if (typeof envelope.operation_id !== "string" || !DOMAIN_BODY.test(envelope.operation_id.slice(3)) || !envelope.operation_id.startsWith("op_")) {
    return `operation_id is not a valid op_ identifier: ${JSON.stringify(envelope.operation_id).slice(0, 72)}`;
  }
  if (typeof envelope.issued_at !== "number" || !Number.isFinite(envelope.issued_at)) {
    return "issued_at must be a finite millisecond timestamp";
  }
  if (envelope.payload !== null && envelope.payload !== undefined) {
    try {
      const encoded = JSON.stringify(envelope.payload);
      if (encoded !== null && encoded.length > LIMITS.payloadBytes) {
        return "payload exceeds the envelope byte limit";
      }
    } catch {
      return "payload is not JSON-encodable";
    }
  }
  return validatePayload(String(envelope.kind), envelope.payload);
}
