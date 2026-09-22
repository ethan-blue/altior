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
