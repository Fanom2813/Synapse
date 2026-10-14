//! M2 providers-and-chat acceptance test (see docs/MILESTONES.md §M2).
//!
//! Offline end-to-end through the real crates: models.dev catalog, provider
//! settings, secret store, single-agent chat (`ChatState`) driven by the
//! scripted `MockLlm`, and usage × catalog cost. No live keys, no network.

use synapse_app::chat_view::{fenced_code_blocks, ChatState};
use synapse_app::settings_providers::{search_providers, ModelRole, ProvidersSettings};
use synapse_llm::keys::MemorySecretStore;
use synapse_llm::{
    AgentEvent, Catalog, ConversationCost, MessageCost, MockLlm, ModelBackend, TurnMessage,
    TurnRequest, Usage,
};

const REPLY_MD: &str = "# Kampala fees\n\nMobile money costs **1%**:\n\n```rust\nfn fee(x: u64) -> u64 { x / 100 }\n```\n";

fn priced_selection(catalog: &Catalog) -> (String, String) {
    for provider in catalog.providers() {
        for (id, model) in &provider.models {
            if model.cost.input.unwrap_or(0.0) > 0.0 {
                return (provider.id.clone(), id.clone());
            }
        }
    }
    panic!("embedded catalog must contain a priced model");
}

#[test]
fn m02_provider_model_selection_and_settings() {
    let catalog = Catalog::embedded().expect("embedded catalog loads");
    assert!(catalog.provider_ids().len() >= 10);
    assert!(catalog.model_count() >= 100);

    // Provider/model selection through the typed catalog API.
    let (provider_id, model_id) = priced_selection(&catalog);
    let provider = catalog.provider(&provider_id).expect("provider exists");
    assert!(!provider.models.is_empty());
    let model = catalog
        .model(&provider_id, &model_id)
        .expect("model exists");
    assert_eq!(model.id, model_id);
    assert!(!catalog.models(&provider_id).expect("models").is_empty());
    assert!(catalog
        .search_models(&model_id, 5)
        .iter()
        .any(|(p, m)| { *p == provider_id.as_str() && m.id == model_id }));
    let _ = catalog
        .capabilities(&provider_id, &model_id)
        .expect("capabilities");
    let price = catalog
        .estimate_cost(&provider_id, &model_id, 1_000_000, 500_000)
        .expect("price");
    assert!(price > 0.0);

    // Provider settings: enable, keys (secret store only), defaults per role.
    let store = synapse_store::Store::open_in_memory().expect("store");
    let secrets = MemorySecretStore::new();
    let mut settings = ProvidersSettings::load(&store).expect("load defaults");
    assert!(!settings.is_enabled(&provider_id));

    settings
        .set_enabled(&catalog, &provider_id, true)
        .expect("enable provider");
    assert!(settings.is_enabled(&provider_id));
    assert!(settings
        .set_enabled(&catalog, "no-such-provider", true)
        .is_err());

    let key = "sk-m02-test-key-abcdef1234567890";
    settings
        .set_key(&secrets, &provider_id, key)
        .expect("store key");
    assert!(settings.has_key(&secrets, &provider_id));

    // Searchable catalog list shows masked keys, never values.
    let rows = search_providers(&catalog, &settings, &secrets, &provider_id);
    let row = rows
        .iter()
        .find(|r| r.id == provider_id)
        .expect("provider row listed");
    assert!(row.enabled && row.has_key);
    let masked = row.key_masked.clone().expect("masked key shown");
    assert!(!masked.contains(key));

    // Default models per role (lead/member/selector), validated by catalog.
    settings
        .set_default(&catalog, ModelRole::Lead, &provider_id, &model_id)
        .expect("lead default");
    settings
        .set_default(&catalog, ModelRole::Member, &provider_id, &model_id)
        .expect("member default");
    settings
        .set_default(&catalog, ModelRole::Selector, &provider_id, &model_id)
        .expect("selector default");
    assert_eq!(
        settings.default_for(ModelRole::Lead),
        Some((provider_id.clone(), model_id.clone()))
    );
    assert!(settings
        .set_default(&catalog, ModelRole::Lead, &provider_id, "no-such-model")
        .is_err());

    // Custom OpenAI-compatible base URL override wins over the catalog URL.
    settings.set_custom_base_url(&provider_id, "http://127.0.0.1:1234/v1");
    assert_eq!(
        settings.base_url_for(&catalog, &provider_id).as_deref(),
        Some("http://127.0.0.1:1234/v1")
    );

    // Persist + reload; the DB holds refs/flags, never the secret value.
    settings.save(&store).expect("save settings");
    let reloaded = ProvidersSettings::load(&store).expect("reload settings");
    assert_eq!(reloaded, settings);
    let raw_enabled = store
        .setting_get("providers.enabled")
        .expect("read raw")
        .unwrap_or_default();
    assert!(!raw_enabled.contains(key));
    let raw_defaults = store
        .setting_get("models.defaults")
        .expect("read raw")
        .unwrap_or_default();
    assert!(!raw_defaults.contains(key));

    // Removing the provider key deletes the secret.
    settings
        .remove_key(&secrets, &provider_id)
        .expect("remove key");
    assert!(!settings.has_key(&secrets, &provider_id));
}

#[tokio::test]
async fn m02_chat_streams_markdown_stop_keeps_draft_and_costs_usage() {
    let catalog = Catalog::embedded().expect("embedded catalog");
    let (provider_id, model_id) = priced_selection(&catalog);

    // Scripted assistant turn: markdown reply + reported usage.
    let usage = Usage::new(120, 60);
    let scripted = synapse_llm::TurnOutput {
        text: REPLY_MD.to_string(),
        tool_calls: Vec::new(),
        usage,
    };
    let backend = MockLlm::with_outputs(vec![scripted]);

    // Send a prompt through the chat: streamed markdown appears.
    let mut chat = ChatState::new();
    chat.set_draft("What are mobile-money fees in Kampala?");
    let (agent_id, req) = chat.send_request("m02-model").expect("send");
    assert_eq!(chat.draft(), "");
    assert!(chat.is_running());
    assert_eq!(chat.send_label(), "Stop");
    assert!(req.messages.iter().any(|m| m.text.contains("Kampala")));
    chat.drive_turn(&backend, agent_id, req).await;
    assert!(!chat.is_running());
    assert!(chat.last_error().is_none());
    let body = chat.message(agent_id).expect("agent message").body.clone();
    assert_eq!(body, REPLY_MD);
    assert!(body.contains("**1%**"));
    // Code highlighting path: one fenced rust block streamed in.
    let blocks = fenced_code_blocks(&body);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].language, "rust");
    assert_eq!(backend.seen_requests().len(), 1);

    // Reported usage prices a conversation cost for the header.
    let events = backend
        .turn(TurnRequest::new("m02-model").with_message(TurnMessage::user("ping")))
        .await
        .expect("mock turn");
    let _ = events;
    // Usage comes back on the stream as well.
    let backend2 = MockLlm::with_outputs(vec![synapse_llm::TurnOutput {
        text: "ok ".to_string(),
        tool_calls: Vec::new(),
        usage,
    }]);
    let stream = backend2
        .turn_stream(TurnRequest::new("m02-model").with_message(TurnMessage::user("ping")))
        .await
        .expect("mock stream");
    let seen_usage = stream.iter().find_map(|e| match e {
        AgentEvent::Usage(u) => Some(*u),
        _ => None,
    });
    assert_eq!(seen_usage, Some(usage));

    let cost = MessageCost::new(
        &catalog,
        "m02-msg-1",
        "m02-conv",
        &provider_id,
        &model_id,
        usage,
    );
    assert_eq!(cost.input_tokens, 120);
    assert_eq!(cost.output_tokens, 60);
    let expected = catalog
        .estimate_cost(&provider_id, &model_id, 120, 60)
        .expect("estimate");
    assert!((cost.cost_usd - expected).abs() < 1e-9);
    assert!(cost.cost_usd > 0.0);
    // Storable in the message meta column and recoverable.
    let meta = format!("{{\"cost\": {}}}", cost.meta_json());
    let roundtrip = MessageCost::from_meta("m02-msg-1", "m02-conv", &meta).expect("meta roundtrip");
    assert_eq!(roundtrip, cost);

    let total = ConversationCost::summarize("m02-conv", std::slice::from_ref(&cost));
    assert_eq!(total.messages, 1);
    assert_eq!(total.input_tokens, 120);
    assert!(total.header_label().contains("1 msg"));
    assert!(total.header_label().contains('$'));

    // Persist the conversation + messages; totals rebuild from stored meta.
    let store = synapse_store::Store::open_in_memory().expect("store");
    store
        .create_project(&synapse_store::NewProject {
            id: "m02-p1".into(),
            name: "M2 chat".into(),
            goal: "verify chat".into(),
            folder: "/tmp/synapse-m02".into(),
            lead_agent_id: String::new(),
            template: "custom".into(),
        })
        .expect("project");
    store
        .create_conversation("m02-conv", "m02-p1", "chat", "Fees")
        .expect("conversation");
    store
        .insert_message(&synapse_store::NewMessage {
            id: "m02-user-1".into(),
            conversation_id: "m02-conv".into(),
            run_id: String::new(),
            from_id: "user".into(),
            to_json: "[]".into(),
            kind: "ask".into(),
            reply_to: String::new(),
            thread_id: String::new(),
            body: "What are mobile-money fees in Kampala?".into(),
            meta: "{}".into(),
        })
        .expect("user message");
    store
        .insert_message(&synapse_store::NewMessage {
            id: "m02-msg-1".into(),
            conversation_id: "m02-conv".into(),
            run_id: String::new(),
            from_id: "agent".into(),
            to_json: "[]".into(),
            kind: "answer".into(),
            reply_to: "m02-user-1".into(),
            thread_id: String::new(),
            body: body.clone(),
            meta,
        })
        .expect("agent message");
    let stored = store
        .list_messages("m02-conv", 10, 0)
        .expect("list messages");
    assert_eq!(stored.len(), 2);
    let agent_row = stored
        .iter()
        .find(|m| m.id == "m02-msg-1")
        .expect("agent row");
    let rebuilt_cost = MessageCost::from_meta(agent_row.id.clone(), "m02-conv", &agent_row.meta)
        .expect("agent meta roundtrip");
    assert_eq!(rebuilt_cost, cost);
    let rebuilt: Vec<MessageCost> = vec![rebuilt_cost];
    assert_eq!(ConversationCost::summarize("m02-conv", &rebuilt), total);
}

#[tokio::test]
async fn m02_chat_stop_preserves_draft() {
    let backend = MockLlm::with_text("late reply that must not land");
    let mut chat = ChatState::new();
    chat.set_draft("first question");
    let (agent_id, req) = chat.send_request("m02-model").expect("send");
    // User keeps typing while the run is in flight, then stops it.
    chat.set_draft("follow-up typed while running");
    chat.stop();
    assert!(!chat.is_running());
    assert_eq!(chat.send_label(), "Send");
    chat.drive_turn(&backend, agent_id, req).await;
    // Stopped: late stream discarded, draft preserved.
    assert_eq!(chat.message(agent_id).expect("agent msg").body, "");
    assert_eq!(chat.draft(), "follow-up typed while running");
    assert!(!chat.message(agent_id).expect("agent msg").streaming);
}
