//! Acceptance tests for A14: Memory and Context connected to real product path.
//!
//! Acceptance criteria (TASKS.md §A14):
//! 1. remember -> new thread recall -> inspect `why_selected` -> correct -> new turn only has new content -> forget -> not recalled
//! 2. Real DB persistence across Core shutdown and restart
//! 3. Secret-shaped content rejection leaves 0 journal writes and 0 persistence
//! 4. IPC memory commands dispatched via daemon (list, propose, confirm, reject, correct, forget)

use std::collections::VecDeque;
use std::str::FromStr;

use altior_core::application::{CoreApplication, CoreDaemon};
use altior_core::runtime::{
    BindingProbeOutcome, HarnessError, HarnessEvent, HarnessPromptRequest, HarnessRuntimePort,
    HarnessSessionId, HarnessSessionInfo,
};
use altior_domain::{
    AcpHarnessBinding, AgentProfile, AgentProfileId, BoundedPath, DisplayName, HarnessBindingId,
    HarnessKind, MemoryContent, MemoryDraft, MemoryId, MemoryKind, MemoryListLimit, MemoryMode,
    MemoryProvenance, MemoryScope, MemorySensitivity, MemorySource, MemoryState, OperationId,
    ThreadId, ThreadTitle, TurnId, UnixMillis,
};
use altior_ipc::{LaunchCredentials, mint_launch_token};
use altior_protocol::{
    CapabilitySet, CommandEnvelope, CorrectMemoryCommand, DesktopHello, EnvelopeLimits, EventBody,
    KnownEvent, MemoryListResponseDto, MemoryRecordDto, ProposeMemoryCommand, ProtocolVersion,
    ProtocolVersionRange,
};
use altior_storage::Store;

// ── Test Double: Mock Agent Harness ─────────────────────────────────

#[derive(Debug, Default)]
struct MockHarness {
    next_session: u32,
    sent_prompts: Vec<(HarnessSessionId, HarnessPromptRequest)>,
    events: std::collections::BTreeMap<HarnessSessionId, VecDeque<HarnessEvent>>,
}

impl MockHarness {
    fn new() -> Self {
        Self::default()
    }
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
        let session_id = HarnessSessionId::new(&format!("ses_a14_{n:016}"))
            .unwrap_or_else(|_| HarnessSessionId::new("ses_a14fallback0001").unwrap());
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

const TOKEN_ENTROPY: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

fn fixture_credentials() -> LaunchCredentials {
    LaunchCredentials {
        instance_id: "cor_a14fixture000001".parse().unwrap(),
        launch_token: mint_launch_token(&TOKEN_ENTROPY).unwrap(),
    }
}

fn desktop_hello(credentials: &LaunchCredentials) -> DesktopHello {
    DesktopHello {
        supported_versions: ProtocolVersionRange::try_new(ProtocolVersion::V1, ProtocolVersion::V1)
            .unwrap(),
        desktop_version: "0.1.0".parse().unwrap(),
        capabilities: CapabilitySet::new(),
        launch_token: credentials.launch_token.clone(),
    }
}

fn fixture_profile(id: &str, memory_mode: MemoryMode) -> AgentProfile {
    let now = UnixMillis::from_millis(1_700_000_000_000);
    AgentProfile {
        id: AgentProfileId::from_str(id).unwrap(),
        display_name: DisplayName::try_from("A14 Assistant").unwrap(),
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
        label: DisplayName::try_from("a14-binding").unwrap(),
        command: BoundedPath::try_from("mock_agent").unwrap(),
        args: Vec::new(),
        env_keys: Vec::new(),
        secret_refs: Vec::new(),
        created_at: now,
    }
}

fn now_at(offset_ms: u64) -> UnixMillis {
    UnixMillis::from_millis(1_700_000_000_000 + offset_ms)
}

fn now() -> UnixMillis {
    UnixMillis::from_millis(1_700_000_000_000)
}

fn settle_turn(app: &mut CoreApplication<MockHarness>, thread_id: &ThreadId, now: UnixMillis) {
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
}

fn limits() -> EnvelopeLimits {
    EnvelopeLimits::default()
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_p24_memory_product_lifecycle_journey() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let mut app =
        CoreApplication::open_in_memory(harness, credentials).expect("open in-memory core");

    let profile = fixture_profile("agp_p240000000000000001", MemoryMode::LongTerm);
    let binding = fixture_binding("hsb_p240000000000000001", "agp_p240000000000000001");
    app.configure_agent(&profile, Some(&binding)).unwrap();

    // 1. Remember: explicit user preference (both English and Chinese text)
    let remember_draft = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("用户偏好使用 Rust 编写后端并保持暗黑主题风格 (User prefers Rust backend and dark theme)").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        state: Some(MemoryState::Confirmed),
        confidence: 100,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance::default(),
        expires_at: None,
    };
    let created = app
        .create_memory(&remember_draft, now_at(10))
        .expect("create memory");
    assert_eq!(created.state, MemoryState::Confirmed);
    let original_id = created.memory_id.clone();

    // 2. Propose candidate memory (inferred)
    let candidate_draft = MemoryDraft {
        id: None,
        content: MemoryContent::try_from("用户正在开发 Altior 桌面客户端").unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: None,
        confidence: 80,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Inferred,
        provenance: MemoryProvenance::default(),
        expires_at: None,
    };
    let proposed = app
        .propose_memory(candidate_draft, now_at(20))
        .expect("propose candidate");
    assert_eq!(proposed.state, MemoryState::Candidate);

    // List candidate memories
    let candidates = app
        .list_memories(
            None,
            Some(MemoryState::Candidate),
            MemoryListLimit::try_new(50).unwrap(),
            None,
        )
        .expect("list candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].memory_id, proposed.memory_id);

    // Confirm candidate memory
    let confirmed_candidate = app
        .confirm_memory(&proposed.memory_id, now_at(30))
        .expect("confirm candidate");
    assert_eq!(confirmed_candidate.state, MemoryState::Confirmed);

    // 3. New session recall: create thread, open thread, and start turn
    let thread_id = ThreadId::from_str("thr_p24rec0000000000001").unwrap();
    let turn_id_1 = TurnId::from_str("trn_p24rec0000000000001").unwrap();
    let op_id_1 = OperationId::from_str("op_p24rec00000000000001").unwrap();

    let title = ThreadTitle::try_from("Memory Recall Test").unwrap();
    app.create_thread(
        thread_id.clone(),
        &binding.agent_profile_id,
        Some(&title),
        None,
        now(),
    )
    .unwrap();
    let open = app.open_thread(&thread_id, Some(&binding)).unwrap();
    assert!(open.session_id.is_some());

    app.start_prompt(
        op_id_1,
        thread_id.clone(),
        turn_id_1.clone(),
        "What language and theme do I prefer for backend? 用户 偏好",
        now_at(40),
    )
    .unwrap();

    settle_turn(&mut app, &thread_id, now_at(45));
    let snap_1 = app
        .get_context_snapshot(&turn_id_1)
        .unwrap()
        .expect("snapshot turn 1 must exist");

    // Inspect why_selected and details
    assert!(
        !snap_1.passthrough,
        "should not be passthrough because memory was injected"
    );
    assert_eq!(snap_1.memory_mode, "long_term");
    assert!(!snap_1.memories.is_empty(), "expected recalled memories");

    let recalled_original = snap_1
        .memories
        .iter()
        .find(|m| m.memory_id == original_id)
        .expect("original memory must be recalled");
    assert!(
        !recalled_original.why_selected.is_empty(),
        "why_selected must not be empty"
    );
    assert!(recalled_original.score > 0.0, "score must be positive");
    assert!(recalled_original.tokens > 0, "tokens must be recorded");

    // 4. Correct memory: correct the preference to Go instead of Rust
    let correct_draft = MemoryDraft {
        id: None,
        content: MemoryContent::try_from(
            "用户偏好切换为使用 Go 语言构建高性能后端 (User changed preference to Go)",
        )
        .unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Preference,
        state: Some(MemoryState::Confirmed),
        confidence: 100,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance::default(),
        expires_at: None,
    };
    let corrected = app
        .correct_memory(&original_id, &correct_draft, now_at(50))
        .expect("correct memory");
    assert_eq!(corrected.state, MemoryState::Confirmed);
    let new_id = corrected.memory_id.clone();

    // Verify original is superseded
    let superseded = app
        .get_memory(&original_id)
        .expect("get original")
        .expect("found");
    assert_eq!(superseded.superseded_by, Some(new_id.clone()));

    // 5. New turn recall: only new content is recalled, old superseded content is excluded
    let turn_id_2 = TurnId::from_str("trn_p24rec0000000000002").unwrap();
    let op_id_2 = OperationId::from_str("op_p24rec00000000000002").unwrap();

    app.start_prompt(
        op_id_2,
        thread_id.clone(),
        turn_id_2.clone(),
        "What language do I prefer now for backend? 用户 偏好",
        now_at(60),
    )
    .unwrap();

    settle_turn(&mut app, &thread_id, now_at(65));
    let snap_2 = app
        .get_context_snapshot(&turn_id_2)
        .unwrap()
        .expect("snapshot turn 2 must exist");

    let recalled_ids: Vec<&MemoryId> = snap_2.memories.iter().map(|m| &m.memory_id).collect();
    assert!(
        recalled_ids.contains(&&new_id),
        "new corrected memory must be recalled"
    );
    assert!(
        !recalled_ids.contains(&&original_id),
        "superseded original memory must NOT be recalled"
    );

    // 6. Forget memory: tombstone the new memory
    let forgotten = app
        .forget_memory(&new_id, now_at(70))
        .expect("forget memory");
    assert_eq!(forgotten.state, MemoryState::Forgotten);

    // 7. Subsequent turn: forgotten memory is no longer recalled
    let turn_id_3 = TurnId::from_str("trn_p24rec0000000000003").unwrap();
    let op_id_3 = OperationId::from_str("op_p24rec00000000000003").unwrap();

    app.start_prompt(
        op_id_3,
        thread_id.clone(),
        turn_id_3.clone(),
        "What language do I prefer now for backend? 用户 偏好",
        now_at(80),
    )
    .unwrap();

    settle_turn(&mut app, &thread_id, now_at(85));
    let snap_3 = app
        .get_context_snapshot(&turn_id_3)
        .unwrap()
        .expect("snapshot turn 3 must exist");

    let recalled_ids_3: Vec<&MemoryId> = snap_3.memories.iter().map(|m| &m.memory_id).collect();
    assert!(
        !recalled_ids_3.contains(&&new_id),
        "forgotten memory must NOT be recalled"
    );
    assert!(
        !recalled_ids_3.contains(&&original_id),
        "superseded memory must NOT be recalled"
    );
}

#[test]
fn test_p24_core_restart_persistence() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let db_path = dir.path().join("altior_p24.db");
    let credentials = fixture_credentials();

    let memory_id_str = {
        let store = Store::open(&db_path).expect("open store first time");
        let mut app = CoreApplication::with_store(MockHarness::new(), store, credentials.clone());

        let draft = MemoryDraft {
            id: None,
            content: MemoryContent::try_from("Persistent memory survive restart 跨重启持久记忆")
                .unwrap(),
            scope: MemoryScope::Global,
            kind: MemoryKind::Fact,
            state: Some(MemoryState::Confirmed),
            confidence: 100,
            sensitivity: MemorySensitivity::Normal,
            source: MemorySource::Explicit,
            provenance: MemoryProvenance::default(),
            expires_at: None,
        };
        let created = app.create_memory(&draft, now()).expect("create memory");
        created.memory_id.to_string()
    };

    // Simulate complete process exit and restart with same SQLite file
    {
        let store = Store::open(&db_path).expect("reopen store second time");
        let app = CoreApplication::with_store(MockHarness::new(), store, credentials);

        let mem_id = MemoryId::from_str(&memory_id_str).unwrap();
        let loaded = app
            .get_memory(&mem_id)
            .expect("get memory")
            .expect("must exist");
        assert_eq!(
            loaded.content.as_str(),
            "Persistent memory survive restart 跨重启持久记忆"
        );
        assert_eq!(loaded.state, MemoryState::Confirmed);

        let list = app
            .list_memories(None, None, MemoryListLimit::try_new(50).unwrap(), None)
            .expect("list memories");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].memory_id, mem_id);
    }
}

#[test]
fn test_p24_secret_shaped_content_zero_persistence() {
    let credentials = fixture_credentials();
    let mut app = CoreApplication::open_in_memory(MockHarness::new(), credentials)
        .expect("open in-memory core");

    // Secret-shaped content (e.g. OpenAI / Anthropic / GitHub keys)
    let secret_draft = MemoryDraft {
        id: None,
        content: MemoryContent::try_from(
            "User secret key is sk-ant-api03-0123456789abcdef0123456789abcdef01234567",
        )
        .unwrap(),
        scope: MemoryScope::Global,
        kind: MemoryKind::Fact,
        state: Some(MemoryState::Confirmed),
        confidence: 100,
        sensitivity: MemorySensitivity::Normal,
        source: MemorySource::Explicit,
        provenance: MemoryProvenance::default(),
        expires_at: None,
    };

    let result = app.create_memory(&secret_draft, now());
    assert!(
        result.is_err(),
        "secret-shaped memory creation must be rejected"
    );

    // Verify 0 records in memory table
    let list = app
        .list_memories(None, None, MemoryListLimit::try_new(50).unwrap(), None)
        .expect("list memories");
    assert!(
        list.is_empty(),
        "rejected secret must leave memory table empty"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_p24_ipc_memory_commands_via_daemon() {
    let credentials = fixture_credentials();
    let harness = MockHarness::new();
    let (mut daemon, listener) = CoreDaemon::in_memory(harness, credentials.clone()).unwrap();
    let limits = limits();
    let now = now();

    let mut client = listener.create_client();
    let hello = desktop_hello(&credentials);
    client.send_json(&hello).unwrap();
    daemon.step(now).unwrap();

    let _: altior_protocol::CoreHello = client.recv_json().unwrap().unwrap();
    let _: altior_protocol::CoreGreeting = client.recv_json().unwrap().unwrap();

    client.subscribe("op_p24sub0000000001", None, now).unwrap();
    daemon.step(now).unwrap();

    // 1. Propose memory via IPC command
    let propose_cmd = ProposeMemoryCommand {
        content: "Propose via IPC command 候选记忆".to_string(),
        scope_kind: "global".to_string(),
        scope_target: None,
        kind: "fact".to_string(),
        sensitivity: Some("normal".to_string()),
        confidence: Some(85),
        source: Some("inferred".to_string()),
        thread_id: None,
        turn_id: None,
        excerpt: None,
    };
    let op_1 = OperationId::from_str("op_p24ipc00000000000001").unwrap();
    let env_1 = CommandEnvelope::propose_memory(&propose_cmd, op_1, now, &limits).unwrap();
    client.send_json(&env_1).unwrap();
    daemon.step(now).unwrap();

    let response_event: altior_protocol::EventEnvelope = client.recv_json().unwrap().unwrap();
    let result_val = match response_event.body {
        EventBody::Known(KnownEvent::CommandResult { data, .. }) => data.unwrap().value().clone(),
        other => panic!("expected CommandResult, got {other:?}"),
    };
    let memory_dto: MemoryRecordDto = serde_json::from_value(result_val).unwrap();
    assert_eq!(memory_dto.state, "candidate");
    let created_mem_id = memory_dto.memory_id.clone();

    // 2. List memories via IPC command
    let op_2 = OperationId::from_str("op_p24ipc00000000000002").unwrap();
    let env_2 = CommandEnvelope::list_memories(
        None,
        None,
        Some("candidate".to_string()),
        Some(10),
        None,
        op_2,
        now,
        &limits,
    )
    .unwrap();
    client.send_json(&env_2).unwrap();
    daemon.step(now).unwrap();

    let list_response: altior_protocol::EventEnvelope = client.recv_json().unwrap().unwrap();
    let list_val = match list_response.body {
        EventBody::Known(KnownEvent::CommandResult { data, .. }) => data.unwrap().value().clone(),
        other => panic!("expected CommandResult, got {other:?}"),
    };
    let list_dto: MemoryListResponseDto = serde_json::from_value(list_val).unwrap();
    assert_eq!(list_dto.memories.len(), 1);
    assert_eq!(list_dto.memories[0].memory_id, created_mem_id);

    // 3. Confirm memory via IPC command
    let op_3 = OperationId::from_str("op_p24ipc00000000000003").unwrap();
    let env_3 =
        CommandEnvelope::confirm_memory(created_mem_id.clone(), op_3, now, &limits).unwrap();
    client.send_json(&env_3).unwrap();
    daemon.step(now).unwrap();

    let confirm_response: altior_protocol::EventEnvelope = client.recv_json().unwrap().unwrap();
    let confirm_val = match confirm_response.body {
        EventBody::Known(KnownEvent::CommandResult { data, .. }) => data.unwrap().value().clone(),
        other => panic!("expected CommandResult, got {other:?}"),
    };
    let confirmed_dto: MemoryRecordDto = serde_json::from_value(confirm_val).unwrap();
    assert_eq!(confirmed_dto.state, "confirmed");

    // 4. Correct memory via IPC command
    let op_4 = OperationId::from_str("op_p24ipc00000000000004").unwrap();
    let correct_cmd = CorrectMemoryCommand {
        memory_id: created_mem_id.clone(),
        content: "Corrected memory via IPC 修正后的记忆".to_string(),
        scope_kind: None,
        scope_target: None,
        kind: None,
        sensitivity: None,
    };
    let env_4 = CommandEnvelope::correct_memory(&correct_cmd, op_4, now, &limits).unwrap();
    client.send_json(&env_4).unwrap();
    daemon.step(now).unwrap();

    let correct_response: altior_protocol::EventEnvelope = client.recv_json().unwrap().unwrap();
    let correct_val = match correct_response.body {
        EventBody::Known(KnownEvent::CommandResult { data, .. }) => data.unwrap().value().clone(),
        other => panic!("expected CommandResult, got {other:?}"),
    };
    let corrected_dto: MemoryRecordDto = serde_json::from_value(correct_val).unwrap();
    assert_eq!(corrected_dto.state, "confirmed");
    assert_eq!(
        corrected_dto.content,
        "Corrected memory via IPC 修正后的记忆"
    );
    let new_mem_id = corrected_dto.memory_id.clone();

    // 5. Forget memory via IPC command
    let op_5 = OperationId::from_str("op_p24ipc00000000000005").unwrap();
    let env_5 = CommandEnvelope::forget_memory(new_mem_id.clone(), op_5, now, &limits).unwrap();
    client.send_json(&env_5).unwrap();
    daemon.step(now).unwrap();

    let forget_response: altior_protocol::EventEnvelope = client.recv_json().unwrap().unwrap();
    let forget_val = match forget_response.body {
        EventBody::Known(KnownEvent::CommandResult { data, .. }) => data.unwrap().value().clone(),
        other => panic!("expected CommandResult, got {other:?}"),
    };
    let forgotten_dto: MemoryRecordDto = serde_json::from_value(forget_val).unwrap();
    assert_eq!(forgotten_dto.state, "forgotten");

    // 6. Secret rejection via IPC command produces command_error
    let op_6 = OperationId::from_str("op_p24ipc00000000000006").unwrap();
    let secret_propose = ProposeMemoryCommand {
        content: "sk-ant-api03-0123456789abcdef0123456789abcdef01234567".to_string(),
        scope_kind: "global".to_string(),
        scope_target: None,
        kind: "fact".to_string(),
        sensitivity: None,
        confidence: None,
        source: None,
        thread_id: None,
        turn_id: None,
        excerpt: None,
    };
    let env_6 = CommandEnvelope::propose_memory(&secret_propose, op_6, now, &limits).unwrap();
    client.send_json(&env_6).unwrap();
    daemon.step(now).unwrap();

    let err_response: altior_protocol::EventEnvelope = client.recv_json().unwrap().unwrap();
    match err_response.body {
        EventBody::Known(KnownEvent::CommandError { code, .. }) => {
            assert_eq!(code, "propose_memory_failed");
        }
        other => panic!("expected CommandError, got {other:?}"),
    }
}
