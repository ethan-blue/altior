import type { TranslationDictionary } from "./types";

export function localizeMemoryScope(
  scope: string,
  t: TranslationDictionary,
): string {
  switch (scope) {
    case "global":
      return t.memoryPane.scopeGlobal;
    case "project":
      return t.memoryPane.scopeProject;
    case "person":
      return t.memoryPane.scopePerson;
    case "thread":
      return t.memoryPane.scopeThread;
    default:
      return scope;
  }
}

export function localizeMemoryKind(
  kind: string,
  t: TranslationDictionary,
): string {
  switch (kind) {
    case "fact":
      return t.memoryPane.kindFact;
    case "preference":
      return t.memoryPane.kindPreference;
    case "instruction":
      return t.memoryPane.kindInstruction;
    case "summary":
      return t.memoryPane.kindSummary;
    default:
      return kind;
  }
}

/** Identity document kinds (name/about + shared preference/instruction). */
export function localizeIdentityKind(
  kind: string,
  t: TranslationDictionary,
): string {
  switch (kind) {
    case "name":
      return t.contextPanel.identityKindName;
    case "about":
      return t.contextPanel.identityKindAbout;
    case "preference":
      return t.memoryPane.kindPreference;
    case "instruction":
      return t.memoryPane.kindInstruction;
    default:
      return kind;
  }
}

export function localizeMemoryState(
  state: string,
  t: TranslationDictionary,
): string {
  switch (state) {
    case "confirmed":
      return t.memoryPane.stateConfirmed;
    case "candidate":
      return t.memoryPane.stateCandidate;
    case "superseded":
      return t.memoryPane.stateSuperseded;
    case "forgotten":
      return t.memoryPane.stateForgotten;
    case "rejected":
      return t.memoryPane.stateRejected;
    default:
      return state;
  }
}

export function localizeRuntimeStatus(
  status: string,
  t: TranslationDictionary,
): string {
  switch (status) {
    case "ready":
      return t.inspector.runtimeReady;
    case "busy":
      return t.inspector.runtimeBusy;
    case "degraded":
      return t.inspector.runtimeDegraded;
    case "shutting_down":
      return t.inspector.runtimeShuttingDown;
    default:
      return status;
  }
}

export function localizeToolStatus(
  status: string,
  t: TranslationDictionary,
): string {
  switch (status) {
    case "completed":
      return t.markdown.toolStatusCompleted;
    case "failed":
      return t.markdown.toolStatusFailed;
    case "running":
      return t.markdown.toolStatusRunning;
    default:
      return status;
  }
}

export function localizeTimelineRowKind(
  kind: string,
  t: TranslationDictionary,
): string {
  switch (kind) {
    case "user-message":
    case "user":
      return t.timeline.you;
    case "assistant-message":
    case "assistant":
      return t.timeline.assistant;
    case "tool":
      return t.timeline.tool;
    case "permission":
    case "approval":
      return t.timeline.approval;
    case "error":
    case "failed":
      return t.timeline.failed;
    case "unknown":
      return t.timeline.unknown;
    default:
      return kind;
  }
}

export function localizePermissionDecision(
  decision: string,
  t: TranslationDictionary,
): string {
  switch (decision) {
    case "approved":
      return t.inspector.decisionApproved;
    case "denied":
      return t.inspector.decisionDenied;
    case "allow":
      return t.inspector.decisionAllow;
    case "deny":
      return t.inspector.decisionDeny;
    case "pending":
      return t.inspector.decisionPending;
    default:
      return decision;
  }
}

export function localizeDropReason(
  reason: string,
  t: TranslationDictionary,
): string {
  switch (reason) {
    case "budget_exhausted":
      return t.contextPanel.dropReasonBudgetExhausted;
    case "scope_disallowed":
      return t.contextPanel.dropReasonScopeDisallowed;
    default:
      return reason;
  }
}

export function localizeCapabilitySupport(
  support: string,
  t: TranslationDictionary,
): string {
  switch (support) {
    case "supported":
      return t.onboarding.capabilitySupported;
    case "unsupported":
      return t.onboarding.capabilityUnsupported;
    default:
      return support;
  }
}

export function localizeDegradedCode(
  code: string,
  t: TranslationDictionary,
): string {
  switch (code) {
    case "retrieval_error":
      return t.contextPanel.degradedCodeRetrievalError;
    case "memory_query_truncated":
      return t.contextPanel.degradedCodeMemoryQueryTruncated;
    case "identity_budget_exceeded":
      return t.contextPanel.degradedCodeIdentityBudgetExceeded;
    case "rendered_prompt_omitted":
      return t.contextPanel.degradedCodeRenderedPromptOmitted;
    default:
      return code;
  }
}

/** Cheap Desktop mapper for protocol why_selected explain strings. */
export function localizeWhySelected(
  raw: string,
  t: TranslationDictionary,
): string {
  return raw
    .split("matched terms:")
    .join(t.contextPanel.whyFragMatchedTerms)
    .split("matched query with score")
    .join(t.contextPanel.whyFragMatchedQuery)
    .split("algorithm:")
    .join(t.contextPanel.whyFragAlgorithm)
    .split("fts_rank:")
    .join(t.contextPanel.whyFragFtsRank)
    .split("scope_weight:")
    .join(t.contextPanel.whyFragScopeWeight)
    .split("confidence:")
    .join(t.contextPanel.whyFragConfidence)
    .split("recency:")
    .join(t.contextPanel.whyFragRecency)
    .split("explicit_bonus:")
    .join(t.contextPanel.whyFragExplicitBonus)
    .split("explicit boost:")
    .join(t.contextPanel.whyFragExplicitBoost)
    .split("explicit +")
    .join(t.contextPanel.whyFragExplicitPlus)
    .split("total:")
    .join(t.contextPanel.whyFragTotal)
    .split("bm25:")
    .join(t.contextPanel.whyFragBm25)
    .split("bm25 ")
    .join(t.contextPanel.whyFragBm25 + " ");
}
