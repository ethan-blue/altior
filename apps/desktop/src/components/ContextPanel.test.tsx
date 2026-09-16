import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ContextSnapshotDto } from "../ipc/dto/ContextSnapshotDto";
import { ContextPanel } from "./ContextPanel";

function fixtureSnapshot(overrides?: Partial<ContextSnapshotDto>): ContextSnapshotDto {
  return {
    turn_id: "trn_p22panel00000000000000001",
    thread_id: "thr_p22panel00000000000000001",
    memory_mode: "long_term",
    created_at: 2000,
    passthrough: false,
    budget: {
      identity_limit_tokens: 512,
      memory_limit_tokens: 1024,
      prompt_tokens: 24,
      identity_tokens: 40,
      memory_tokens: 88,
      total_tokens: 152,
    },
    identity: [{ document_id: "idt_p22panel00000000000000001", kind: "name", tokens: 40 }],
    memories: [
      {
        memory_id: "mem_p22panel00000000000000001",
        kind: "preference",
        scope_kind: "global",
        scope_target: null,
        confidence: 100,
        explicit: true,
        tokens: 88,
        score: 0.92,
        why_selected: "matched terms: rust; bm25 0.42; explicit +0.2",
        provenance_thread_id: "thr_p22origin0000000000000001",
        provenance_turn_id: "trn_p22origin0000000000000001",
      },
    ],
    dropped: [
      {
        memory_id: "mem_p22panel00000000000000002",
        tokens: 6000,
        rank: 2,
        reason: "budget_exhausted",
      },
    ],
    degraded: null,
    rendered_prompt: null,
    ...overrides,
  } as ContextSnapshotDto;
}

describe("ContextPanel (P2.2 inspector)", () => {
  it("shows an empty state when no snapshot is recorded", () => {
    render(<ContextPanel snapshot={null} />);
    expect(screen.getByTestId("context-empty")).toBeTruthy();
  });

  it("shows loading state when context snapshot is loading", () => {
    render(<ContextPanel snapshot={null} status="loading" />);
    expect(screen.getByTestId("context-loading")).toBeTruthy();
  });

  it("shows error state with error message when snapshot retrieval fails", () => {
    render(<ContextPanel snapshot={null} status="error" error="Network timeout" />);
    const err = screen.getByTestId("context-error");
    expect(err).toBeTruthy();
    expect(err.textContent).toContain("Network timeout");
  });

  it("shows empty state when status is not_found", () => {
    render(<ContextPanel snapshot={null} status="not_found" />);
    expect(screen.getByTestId("context-empty")).toBeTruthy();
  });

  it("shows a memory-off state when the turn ran with memory disabled", () => {
    render(<ContextPanel snapshot={fixtureSnapshot({ memory_mode: "off" })} />);
    expect(screen.getByTestId("context-memory-off")).toBeTruthy();
  });

  it("renders token budget accounting", () => {
    render(<ContextPanel snapshot={fixtureSnapshot()} />);

    const budget = screen.getByTestId("context-budget");
    expect(budget.textContent).toContain("512");
    expect(budget.textContent).toContain("88");
    expect(budget.textContent).toContain("152");
  });

  it("renders selected memory rationale and provenance", () => {
    render(<ContextPanel snapshot={fixtureSnapshot()} />);

    const why = screen.getByTestId("memory-why-selected");
    expect(why.textContent).toContain("matched terms: rust");

    const provenance = screen.getByTestId("memory-provenance");
    expect(provenance.textContent).toContain("thr_p22origin0000000000000001");
    expect(provenance.textContent).toContain("trn_p22origin0000000000000001");

    expect(screen.getByTestId("context-memory-mem_p22panel00000000000000001")).toBeTruthy();
  });

  it("renders dropped entries with reasons", () => {
    render(<ContextPanel snapshot={fixtureSnapshot()} />);

    const dropped = screen.getByTestId("context-dropped-list");
    expect(dropped.textContent).toContain("mem_p22panel00000000000000002");
    expect(dropped.textContent).toContain("budget_exhausted");
  });

  it("renders a degradation badge when assembly degraded", () => {
    render(
      <ContextPanel
        snapshot={fixtureSnapshot({
          degraded: { code: "retrieval_error", detail: "fts unavailable" },
        })}
      />,
    );

    const badge = screen.getByTestId("context-degraded-badge");
    expect(badge.textContent).toContain("retrieval_error");
  });

  it("hides dropped and identity sections when empty", () => {
    render(
      <ContextPanel
        snapshot={fixtureSnapshot({ dropped: [], identity: [] })}
      />,
    );

    expect(screen.queryByTestId("context-dropped")).toBeNull();
    expect(screen.queryByTestId("context-identity")).toBeNull();
  });
});
