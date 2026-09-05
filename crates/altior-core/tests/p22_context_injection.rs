//! P2.2 acceptance: identity documents and confirmed-memory context injection.
//!
//! Verifies against a real `CoreApplication` with a `MockHarness`:
//! 1. A confirmed fact is recalled in a *new* ACP thread via deterministic
//!    context assembly, and the recorded `ContextSnapshot` explains the
//!    selection (score, rationale, provenance, token budget).
//! 2. With no identity documents and no memories, the wire prompt is
//!    byte-identical to the user prompt (passthrough invariant).
//! 3. Correction supersedes the original for future turns while journal
//!    history is preserved.
//! 4. Forgetting removes a memory from future context.
//! 5. Secret-shaped memory content is rejected before any durable write.
//! 6. An identity document is injected into the wire prompt and audited.

use std::collections::VecDeque;
use std::str::FromStr;

use altior_core::application::CoreApplication;
use altior_core::runtime::{
    BindingProbeOutcome, HarnessError, HarnessEvent, HarnessPromptRequest, HarnessRuntimePort,
    HarnessSessionId, HarnessSessionInfo, TurnAdmission,
};
use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileId, BoundedPath, DisplayName, HarnessBindingId,
    HarnessKind, MemoryContent, MemoryDraft, MemoryExcerpt, MemoryKind, MemoryMode,
    MemoryProvenance, MemoryScope, MemorySensitivity, MemorySource, OperationId, ThreadId,
    ThreadTitle, TurnId, UnixMillis,
};
use altior_domain::{IdentityContent, IdentityDocument, IdentityDocumentId, IdentityDocumentKind};
use altior_ipc::{LaunchCredentials, mint_launch_token};

// ── Test Double: Mock Agent Harness ─────────────────────────────────

#[derive(Debug, Default)]
struct MockHarness {
    next_session: u32,
    sent_prompts: Vec<(HarnessSessionId, HarnessPromptRequest)>,
    events: std::collections::BTreeMap<HarnessSessionId, VecDeque<HarnessEvent>>,
}

impl MockHarness {
    fn queue_event(&mut self, session_id: &HarnessSessionId, event: HarnessEvent) {
        self.events
            .entry(session_id.clone())
            .or_default()
            .push_back(event);
    }
}

impl HarnessRuntimePort for MockHarness {
    fn probe_binding(
        &mut self,
        _binding: &AcpHarnessBinding,
    ) -> Result<BindingProbeOutcome, HarnessError> {
        Ok(BindingProbeOutcome {
            ok: true,
            capabilities: altior_protocol::CapabilitySet::new(),
            diagnostics: None,
        })
    }

    fn create_session(
        &mut self,
        _binding: &AcpHarnessBinding,
        _thread_id: &ThreadId,
        _project: Option<&altior_domain::ProjectRef>,
    ) -> Result<HarnessSessionInfo, HarnessError> {
        let n = self.next_session;
        self.next_session = n.saturating_add(1);
        let session_id = HarnessSessionId::new(&format!("ses_p22_{n:016}"))
            .unwrap_or_else(|_| HarnessSessionId::new("ses_p22fallback0001").unwrap());
        Ok(HarnessSessionInfo {
            session_id,
            capabilities: altior_protocol::CapabilitySet::new(),
        })
    }

    fn resume_session(
        &mut self,
        _binding: &AcpHarnessBinding,
        session_id: &HarnessSessionId,
        _thread_id: &ThreadId,
    ) -> Result<HarnessSessionInfo, HarnessError> {
        Ok(HarnessSessionInfo {
            session_id: session_id.clone(),
            capabilities: altior_protocol::CapabilitySet::new(),
        })
    }

    fn send_prompt(
        &mut self,
        session_id: &HarnessSessionId,
        prompt: HarnessPromptRequest,
    ) -> Result<(), HarnessError> {
        self.sent_prompts.push((session_id.clone(), prompt));
        Ok(())
    }

    fn cancel_turn(&mut self, _session_id: &HarnessSessionId) -> Result<(), HarnessError> {
        Ok(())
    }

    fn decide_permission(
        &mut self,
        _session_id: &HarnessSessionId,
        _event_id: &altior_domain::EventId,
        _decision: altior_domain::PermissionDecision,
    ) -> Result<(), HarnessError> {
        Ok(())
    }

    fn poll_event(
        &mut self,
        session_id: &HarnessSessionId,
    ) -> Result<Option<HarnessEvent>, HarnessError> {
        Ok(self
            .events
            .get_mut(session_id)
            .and_then(std::collections::VecDeque::pop_front))
    }

    fn close_session(&mut self, _session_id: &HarnessSessionId) -> Result<(), HarnessError> {
        Ok(())
    }
}

// ── Fixtures ────────────────────────────────────────────────────────

const TOKEN_ENTROPY: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

fn fixture_credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: "cor_p22fixture000001".parse().unwrap(),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).unwrap(),
    }
}

fn fixture_profile(id: &str, memory_mode: MemoryMode) -> AgentProfile {
    let now = UnixMillis::from_millis(1_700_000_000_000);
    AgentProfile {
        id: AgentProfileId::from_str(id).unwrap(),
        display_name: DisplayName::try_from("P22 Agent").unwrap(),
        preferred_harness: HarnessKind::Acp,
        memory_mode,
        created_at: now,
        updated_at: now,
    }
}

fn fixture_binding(id: &str, profile_id: &str) -> AcpHarnessBinding {
    let now = UnixMillis::from_millis(1_700_000_000_000);
    AcpHarnessBinding {
        id: HarnessBindingId::from_str(id).unwrap(),
        agent_profile_id: AgentProfileId::from_str(profile_id).unwrap(),
        label: DisplayName::try_from("p22-agent").unwrap(),
        command: BoundedPath::try_from("mock_agent").unwrap(),
        args: Vec::new(),
        env_keys: Vec::new(),
        secret_refs: Vec::new(),
        created_at: now,
    }
}

fn fixture_draft(content: &str) -> MemoryDraft {
    MemoryDraft {
        id: None,
        content: MemoryContent::try_from(content).expect("valid memory content"),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 90,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance {
            thread_id: None,
            turn_id: None,
            excerpt: Some(MemoryExcerpt::try_from("user stated preference").unwrap()),
        },
        expires_at: None,
    }
}

fn fixture_doc(n: u32, kind: IdentityDocumentKind, content: &str) -> IdentityDocument {
    IdentityDocument {
        id: IdentityDocumentId::from_str(&format!("idd_p22doc{n:010}")).unwrap(),
        kind,
        content: IdentityContent::try_from(content).expect("valid identity content"),
        created_at: UnixMillis::from_millis(1_700_000_000_100),
        updated_at: UnixMillis::from_millis(1_700_000_000_100),
    }
}

/// Zero-pads a turn suffix so identifier bodies meet the 16-char minimum.
fn pad_suffix(s: &str) -> String {
    format!("{s:0>9}")
}

/// Creates the app, agent, and an opened thread; returns (app, `thread_id`).
fn app_with_thread(
    profile_id: &str,
    binding_id: &str,
    mode: MemoryMode,
) -> (CoreApplication<MockHarness>, ThreadId) {
    let mut app =
        CoreApplication::open_in_memory(MockHarness::default(), fixture_credentials()).unwrap();
    let profile = fixture_profile(profile_id, mode);
    let binding = fixture_binding(binding_id, profile_id);
    app.configure_agent(&profile, Some(&binding)).unwrap();

    let thread_id = ThreadId::from_str("thr_p22fixture000001").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);
    let title = ThreadTitle::try_from("P2.2 Context Test").unwrap();
    app.create_thread(
        thread_id.clone(),
        &binding.agent_profile_id,
        Some(&title),
        None,
        now,
    )
    .unwrap();
    let open = app.open_thread(&thread_id, Some(&binding)).unwrap();
    assert!(open.session_id.is_some());
    (app, thread_id)
}

fn run_turn(
    app: &mut CoreApplication<MockHarness>,
    thread_id: &ThreadId,
    turn_suffix: &str,
    content: &str,
) -> TurnAdmission {
    let now = UnixMillis::from_millis(1_700_000_001_000);
    let op = OperationId::from_str(&format!("op_p22turn{}", pad_suffix(turn_suffix))).unwrap();
    let turn = TurnId::from_str(&format!("trn_p22turn{}", pad_suffix(turn_suffix))).unwrap();
    let admission = app
        .start_prompt(op, thread_id.clone(), turn.clone(), content, now)
        .unwrap_or_else(|e| panic!("start_prompt failed: {e:?}"));

    // Settle the turn so the same thread can accept the next one.
    let session = app
        .supervisor_mut()
        .harness_mut()
        .sent_prompts
        .last()
        .map(|(s, _)| s.clone())
        .expect("at least one prompt sent");
    app.supervisor_mut()
        .harness_mut()
        .queue_event(&session, HarnessEvent::Completed { payload: None });
    app.poll_thread_events(thread_id, now).unwrap().unwrap();
    admission
}

fn last_wire_prompt(app: &mut CoreApplication<MockHarness>) -> String {
    app.supervisor_mut()
        .harness_mut()
        .sent_prompts
        .last()
        .map(|(_, p)| p.prompt.clone())
        .expect("a wire prompt was sent")
}

// ── Test 1: Cross-thread recall with explainable snapshot ──────────

#[test]
fn confirmed_fact_recalled_in_new_thread_with_explainable_snapshot() {
    let (mut app, thread_a) = app_with_thread(
        "agp_p22fixture000001",
        "hsb_p22fixture000001",
        MemoryMode::LongTerm,
    );

    // Seed one confirmed global memory before any turn.
    let seeded = app
        .store_mut()
        .create_memory(
            &fixture_draft("User prefers Rust over Python"),
            UnixMillis::from_millis(1_700_000_000_500),
        )
        .expect("seed confirmed memory");
    assert_eq!(seeded.state, altior_domain::MemoryState::Confirmed);

    // Turn on thread A: the memory must be injected into the wire prompt.
    run_turn(
        &mut app,
        &thread_a,
        "01a",
        "What programming language do I prefer?",
    );
    let wire_a = last_wire_prompt(&mut app);
    assert!(
        wire_a.contains("User prefers Rust over Python"),
        "memory must be injected into wire prompt, got: {wire_a}"
    );
    assert!(
        wire_a.contains("What programming language do I prefer?"),
        "user prompt text must be preserved in wire prompt"
    );

    // Snapshot is recorded and explains the selection.
    let turn_a = TurnId::from_str(&format!("trn_p22turn{}", pad_suffix("01a"))).unwrap();
    let snapshot = app
        .get_context_snapshot(&turn_a)
        .expect("snapshot lookup")
        .expect("snapshot must exist for turn");
    assert!(!snapshot.passthrough);
    assert_eq!(snapshot.memories.len(), 1);
    let entry = &snapshot.memories[0];
    assert_eq!(entry.memory_id, seeded.memory_id);
    assert!(
        !entry.why_selected.is_empty(),
        "why_selected must explain the pick"
    );
    assert!(entry.score > 0.0);
    assert!(snapshot.budget.memory_tokens > 0);
    assert!(snapshot.budget.total_tokens >= snapshot.budget.prompt_tokens);

    // NEW thread B recalls the same confirmed fact (cross-thread recall).
    let binding = fixture_binding("hsb_p22fixture000001", "agp_p22fixture000001");
    let thread_b = ThreadId::from_str("thr_p22fixture000002").unwrap();
    let now = UnixMillis::from_millis(1_700_000_000_000);
    app.create_thread(
        thread_b.clone(),
        &binding.agent_profile_id,
        Some(&ThreadTitle::try_from("P2.2 Thread B").unwrap()),
        None,
        now,
    )
    .unwrap();
    app.open_thread(&thread_b, Some(&binding)).unwrap();
    run_turn(
        &mut app,
        &thread_b,
        "01b",
        "Which language do I prefer for examples?",
    );
    let wire_b = last_wire_prompt(&mut app);
    assert!(
        wire_b.contains("User prefers Rust over Python"),
        "confirmed fact must be recalled in a new thread"
    );
}

// ── Test 2: Passthrough invariant ───────────────────────────────────

#[test]
fn empty_context_is_byte_identical_passthrough() {
    let (mut app, thread_id) = app_with_thread(
        "agp_p22fixture000002",
        "hsb_p22fixture000002",
        MemoryMode::LongTerm,
    );
    // No memories, no identity documents.
    run_turn(&mut app, &thread_id, "02a", "Just a plain question");

    let wire = last_wire_prompt(&mut app);
    assert_eq!(
        wire, "Just a plain question",
        "no context means wire prompt is byte-identical to user prompt"
    );

    let turn = TurnId::from_str(&format!("trn_p22turn{}", pad_suffix("02a"))).unwrap();
    let snapshot = app
        .get_context_snapshot(&turn)
        .expect("snapshot lookup")
        .expect("snapshot exists");
    assert!(snapshot.passthrough);
    assert!(snapshot.memories.is_empty());
    assert!(snapshot.identity.is_empty());
    assert_eq!(snapshot.budget.memory_tokens, 0);
    assert_eq!(snapshot.budget.identity_tokens, 0);
}

// ── Test 3: Correction supersedes injection, history preserved ──────

#[test]
fn correction_supersedes_injection_and_preserves_history() {
    let (mut app, thread_id) = app_with_thread(
        "agp_p22fixture000003",
        "hsb_p22fixture000003",
        MemoryMode::LongTerm,
    );

    let original = app
        .store_mut()
        .create_memory(
            &fixture_draft("User prefers Rust over Python"),
            UnixMillis::from_millis(1_700_000_000_500),
        )
        .unwrap();

    run_turn(&mut app, &thread_id, "03a", "What language do I prefer?");
    assert!(last_wire_prompt(&mut app).contains("prefers Rust"));

    // Correct: the replacement supersedes the original atomically.
    app.store_mut()
        .correct_memory(
            &original.memory_id,
            &fixture_draft("User prefers Go over Python"),
            UnixMillis::from_millis(1_700_000_002_000),
        )
        .expect("correct memory");

    run_turn(
        &mut app,
        &thread_id,
        "03b",
        "What language do I prefer again?",
    );
    let wire = last_wire_prompt(&mut app);
    assert!(
        wire.contains("prefers Go"),
        "corrected content must be injected, got: {wire}"
    );
    assert!(
        !wire.contains("prefers Rust"),
        "superseded content must not be injected, got: {wire}"
    );

    // Journal history preserves both events.
    let rows = app
        .store_mut()
        .domain_journal_records(0, altior_storage::JournalLimit::try_new(50).unwrap())
        .expect("read domain journal");
    let all_kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
    assert!(
        all_kinds.contains(&"memory.superseded"),
        "journal must retain the supersede event, got: {all_kinds:?}"
    );
    assert_eq!(
        all_kinds
            .iter()
            .filter(|k| **k == "memory.confirmed")
            .count(),
        2,
        "original and replacement confirmations must both be journaled"
    );
}

// ── Test 4: Forgetting removes memory from future context ──────────

#[test]
fn forgetting_removes_memory_from_future_context() {
    let (mut app, thread_id) = app_with_thread(
        "agp_p22fixture000004",
        "hsb_p22fixture000004",
        MemoryMode::LongTerm,
    );

    let seeded = app
        .store_mut()
        .create_memory(
            &fixture_draft("User's favorite editor is Neovim"),
            UnixMillis::from_millis(1_700_000_000_500),
        )
        .unwrap();

    run_turn(&mut app, &thread_id, "04a", "Which editor do I use?");
    assert!(last_wire_prompt(&mut app).contains("Neovim"));

    app.store_mut()
        .forget_memory(
            &seeded.memory_id,
            UnixMillis::from_millis(1_700_000_002_000),
        )
        .expect("forget memory");

    run_turn(&mut app, &thread_id, "04b", "Which editor do I use now?");
    let wire = last_wire_prompt(&mut app);
    assert!(
        !wire.contains("Neovim"),
        "forgotten memory must not be injected, got: {wire}"
    );

    let turn = TurnId::from_str(&format!("trn_p22turn{}", pad_suffix("04b"))).unwrap();
    let snapshot = app
        .get_context_snapshot(&turn)
        .expect("snapshot lookup")
        .expect("snapshot exists");
    assert!(snapshot.passthrough);
}

// ── Test 5: Secret-shaped content rejected before durable writes ────

#[test]
fn secret_shaped_memory_rejected_before_journal_and_projection() {
    let (mut app, _thread_id) = app_with_thread(
        "agp_p22fixture000005",
        "hsb_p22fixture000005",
        MemoryMode::LongTerm,
    );

    // Assemble the credential-shaped fixture at runtime so no literal
    // secret ever exists in source control; the detector must still
    // recognize the assembled shape.
    let secret = format!("my key is {}ant-api03-{}", "sk-", "0123456789abcdefghij");
    let secret_draft = MemoryDraft {
        content: MemoryContent::try_from(secret.as_str()).unwrap(),
        ..fixture_draft("placeholder")
    };

    let err = app
        .store_mut()
        .create_memory(&secret_draft, UnixMillis::from_millis(1_700_000_000_500));
    assert!(err.is_err(), "secret-shaped memory must be rejected");

    // Zero durable contamination.
    let rows = app
        .store_mut()
        .domain_journal_records(0, altior_storage::JournalLimit::try_new(50).unwrap())
        .expect("read domain journal");
    assert!(
        rows.iter()
            .all(|r| r.kind != "memory.proposed" && r.kind != "memory.confirmed"),
        "no memory events may be journaled for rejected secrets"
    );
    let memories = app
        .store_mut()
        .list_memories(
            None,
            None,
            altior_domain::MemoryListLimit::try_new(50).unwrap(),
            None,
        )
        .expect("list memories");
    assert!(memories.is_empty(), "no memory rows may be projected");
}

// ── Test 6: Identity document injected and audited ──────────────────

#[test]
fn identity_document_injected_into_wire_prompt_and_audited() {
    let (mut app, thread_id) = app_with_thread(
        "agp_p22fixture000006",
        "hsb_p22fixture000006",
        MemoryMode::LongTerm,
    );

    app.store_mut()
        .put_identity_document(&fixture_doc(1, IdentityDocumentKind::Name, "Ada Lovelace"))
        .expect("put identity document");

    run_turn(&mut app, &thread_id, "06a", "Who am I speaking as?");
    let wire = last_wire_prompt(&mut app);
    assert!(
        wire.contains("Ada Lovelace"),
        "identity document must be injected, got: {wire}"
    );

    let turn = TurnId::from_str(&format!("trn_p22turn{}", pad_suffix("06a"))).unwrap();
    let snapshot = app
        .get_context_snapshot(&turn)
        .expect("snapshot lookup")
        .expect("snapshot exists");
    assert_eq!(snapshot.identity.len(), 1);
    assert_eq!(snapshot.identity[0].kind, IdentityDocumentKind::Name);
    assert!(snapshot.budget.identity_tokens > 0);
}
