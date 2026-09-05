import { beforeEach, describe, expect, it } from "vitest";
import { InMemoryTransport } from "../ipc/inMemoryTransport";
import { createApplicationStore } from "./applicationStore";

describe("P2.2 context snapshot & identity document store actions", () => {
  let transport: InMemoryTransport;

  beforeEach(() => {
    transport = new InMemoryTransport();
  });

  it("getContextSnapshot resolves the snapshot and stores it in state", async () => {
    const store = createApplicationStore(transport);
    await store.init();

    const snapshot = await store.getContextSnapshot(null, "trn_p22store0000000000001");

    expect(snapshot).not.toBeNull();
    expect(snapshot?.memory_mode).toBe("long_term");
    expect(snapshot?.budget.total_tokens).toBeGreaterThan(0);
    expect(snapshot?.memories.length).toBeGreaterThan(0);
    expect(snapshot?.memories[0]?.why_selected).toContain("matched terms");
    expect(store.getState().contextSnapshot?.turn_id).toBe("trn_p22store0000000000001");
  });

  it("getContextSnapshot with only a thread id resolves the thread-latest record", async () => {
    const store = createApplicationStore(transport);
    await store.init();

    const snapshot = await store.getContextSnapshot(
      "thr_p22store0000000000000001",
      null,
    );
    expect(snapshot?.thread_id).toBe("thr_p22store0000000000000001");
  });

  it("putIdentityDocument then listIdentityDocuments round-trips through state", async () => {
    const store = createApplicationStore(transport);
    await store.init();

    await store.putIdentityDocument({
      document_id: "idd_p22_test",
      kind: "about",
      content: "I prefer concise answers.",
    });
    const docs = await store.listIdentityDocuments();

    expect(docs.some((d) => d.document_id === "idd_p22_test")).toBe(true);
    expect(
      store.getState().identityDocuments.some((d) => d.document_id === "idd_p22_test"),
    ).toBe(true);
  });

  it("deleteIdentityDocument removes the document from state", async () => {
    const store = createApplicationStore(transport);
    await store.init();

    await store.putIdentityDocument({
      document_id: "idd_p22_gone",
      kind: "name",
      content: "Altior",
    });
    await store.deleteIdentityDocument("idd_p22_gone");
    const docs = await store.listIdentityDocuments();

    expect(docs.some((d) => d.document_id === "idd_p22_gone")).toBe(false);
    expect(
      store.getState().identityDocuments.some((d) => d.document_id === "idd_p22_gone"),
    ).toBe(false);
  });
});
