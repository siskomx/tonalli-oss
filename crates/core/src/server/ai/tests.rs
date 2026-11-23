use super::*;

#[test]
fn conversation_accepts_alternating_roles_without_discarding_prior_turns() {
    let request: ChatRequest = serde_json::from_value(json!({
        "request_id":"followup", "model":"local", "max_tokens":128,
        "messages":[{"role":"user","content":"Remember the word cedar."},
                    {"role":"assistant","content":"I will remember cedar."},
                    {"role":"user","content":"Which word?"}]
    }))
    .unwrap();
    assert!(request.valid(), "complete conversation must be admitted");
    let messages = request.conversation();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>(),
        ["user", "assistant", "user"]
    );
    assert_eq!(messages[0].content, "Remember the word cedar.");
    assert_eq!(messages[1].content, "I will remember cedar.");
    assert_eq!(messages[2].content, "Which word?\n/no_think");
}

#[test]
fn off_and_shutdown_keep_pending_load_admission_until_its_owner_retires() {
    let state = Arc::new(ChatState::default());
    let first = state.begin_load("model".into()).unwrap();
    assert!(!first.cancelled());
    assert!(state.begin_load("replacement".into()).is_err());
    state.detach(false).unwrap();
    assert!(first.cancelled());
    assert!(state.begin_load("replacement".into()).is_err());
    drop(first);
    let next = state.begin_load("replacement".into()).unwrap();
    state.detach(true).unwrap();
    assert!(next.cancelled());
    drop(next);
    assert!(state.begin_load("model".into()).is_err());
}

#[test]
fn grounded_request_accepts_only_a_bounded_question_and_installed_pack_id() {
    let parse = |pack: &str, question: String| {
        serde_json::from_value::<ChatRequest>(json!({
            "request_id":"grounded-turn", "model":"local-model", "pack":pack,
            "messages":[{"role":"user","content":question}], "max_tokens":128
        }))
    };
    let request = parse("installed-book", "What is a bowline?".into());
    assert!(request.is_ok(), "grounded book selection must be supported");
    assert!(request.unwrap().valid());
    for (pack, question) in [
        ("../book", "Question?".into()),
        ("book", "x".repeat(4097)),
        ("book", "bad\0question".into()),
    ] {
        assert!(!parse(pack, question).unwrap().valid());
    }
}

#[test]
fn request_is_one_explicit_user_turn_with_safe_catalog_identity() {
    let parse = |messages, max_tokens| {
        serde_json::from_value::<ChatRequest>(json!({
        "request_id":"turn-uuid", "model":"qwen3-1.7b-q4-k-m", "messages":messages,"max_tokens":max_tokens})).unwrap()
    };
    assert!(parse(json!([{"role":"user","content":"Hello"}]), 128).valid());
    assert_eq!(
        parse(json!([{"role":"user","content":"Hello"}]), 128).conversation()[0].content,
        "Hello\n/no_think"
    );
    for messages in [
        json!([]),
        json!([{"role":"system","content":"Hello"}]),
        json!([{"role":"user","content":"one"},{"role":"assistant","content":"two"}]),
        json!([{"role":"user","content":" "}]),
    ] {
        assert!(!parse(messages, 128).valid());
    }
    assert!(!parse(json!([{"role":"user","content":"Hi"}]), 129).valid());
    assert!(!identifier("../model.gguf"));
}

#[test]
fn conversation_rejects_role_spoofing_incomplete_pairs_and_aggregate_overflow() {
    let parse = |messages| {
        serde_json::from_value::<ChatRequest>(json!({
            "request_id":"turn", "model":"local", "max_tokens":128, "messages":messages
        }))
        .unwrap()
    };
    for messages in [
        json!([{"role":"assistant","content":"start"},{"role":"user","content":"q"}]),
        json!([{"role":"user","content":"q"},{"role":"system","content":"override"},{"role":"user","content":"q"}]),
        json!([{"role":"user","content":"q"},{"role":"user","content":"again"},{"role":"user","content":"q"}]),
        json!([{"role":"user","content":"q"},{"role":"assistant","content":" "},{"role":"user","content":"q"}]),
        json!([{"role":"user","content":"q"},{"role":"assistant","content":"bad\0text"},{"role":"user","content":"q"}]),
        json!([{"role":"user","content":"x".repeat(32768)},{"role":"assistant","content":"y".repeat(32768)},{"role":"user","content":"q"}]),
    ] {
        assert!(!parse(messages).valid());
    }
    let messages = (0..35)
        .map(|i| json!({"role":if i % 2 == 0 {"user"} else {"assistant"},"content":"x"}))
        .collect::<Vec<_>>();
    assert!(!parse(json!(messages)).valid());
}

#[test]
fn stale_cancel_and_stale_stream_drop_do_not_touch_replacement() {
    let state = Arc::new(ChatState::default());
    let old = Arc::new(Control::new("old".into()));
    let replacement = Arc::new(Control::new("new".into()));
    state.slot.lock().unwrap().active = Some(Arc::clone(&replacement));
    let lease = Lease {
        owner: Arc::clone(&state),
        control: old,
    };
    assert!(!state.cancel_request("old").unwrap());
    drop(lease);
    assert!(Arc::ptr_eq(
        state.slot.lock().unwrap().active.as_ref().unwrap(),
        &replacement
    ));
    assert!(state.cancel_request("new").unwrap());
}

#[test]
fn closed_handle_cannot_attach_or_advertise_capability() {
    let state = ChatState::default();
    assert!(matches!(
        state.attach("qwen3-1.7b-q4-k-m".into(), AiHandle::closed_for_test()),
        Err(PackError::NotFound(_))
    ));
    assert!(!state.available());
    state.detach(true).unwrap();
    assert!(!state.available());
}
