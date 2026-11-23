use super::*;

struct Recording(Mutex<Vec<GenerationEvent>>);

impl TokenSink for Recording {
    fn on_event(&self, event: GenerationEvent) -> Result<(), PackError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

#[test]
fn closed_handle_refuses_generation_without_accepting_a_sink() {
    let sink = Arc::new(Recording(Mutex::new(Vec::new())));
    assert!(matches!(
        generate(AiHandle::closed_for_test(), "Hi".into(), 8, sink.clone()),
        Err(PackError::NotFound(_))
    ));
    assert!(sink.0.lock().unwrap().is_empty());
}

#[test]
#[ignore = "requires verified immutable internal Qwen and actual runtime RAM admission"]
fn real_qwen_conversation_recalls_an_earlier_user_turn() {
    let _serial = delivery_serial();
    let path = std::env::var("TONALLI_TEST_QWEN_INTERNAL").unwrap();
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Generation).unwrap();
    let sink = Arc::new(Recording(Mutex::new(Vec::new())));
    generate_chat(
        handle.clone(),
        vec![
            ChatMessage {
                role: "user".into(),
                content: "The test word is CEDAR.".into(),
            },
            ChatMessage {
                role: "assistant".into(),
                content: "Understood.".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "Reply with just the test word stated earlier. /no_think".into(),
            },
        ],
        32,
        sink.clone(),
    )
    .unwrap();
    let start = Instant::now();
    loop {
        if sink
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, GenerationEvent::Finished { .. }))
        {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(90));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let events = sink.0.lock().unwrap();
    assert!(matches!(
        events.last(),
        Some(GenerationEvent::Finished {
            reason: GenerationEnd::Eos | GenerationEnd::Limit,
            error: None,
            ..
        })
    ));
    let text = events
        .iter()
        .filter_map(|event| match event {
            GenerationEvent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    eprintln!("synthetic conversation answer: {text}");
    assert!(
        text.to_ascii_lowercase().contains("cedar"),
        "earlier user content must reach the native template"
    );
    assert!(REGISTRY.lock().unwrap().active.is_none());
    handle.close().unwrap();
}

#[test]
#[ignore = "requires verified immutable internal Qwen and actual runtime RAM admission"]
fn real_qwen_streams_bounded_utf8_and_releases_native_job() {
    let _serial = delivery_serial();
    let path = std::env::var("TONALLI_TEST_QWEN_INTERNAL").unwrap();
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Generation).unwrap();
    let sink = Arc::new(Recording(Mutex::new(Vec::new())));
    let id = generate(
        handle.clone(),
        "Reply with one Spanish greeting. /no_think".into(),
        32,
        sink.clone(),
    )
    .unwrap();
    let start = Instant::now();
    loop {
        if sink
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, GenerationEvent::Finished { .. }))
        {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(90));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let events = sink.0.lock().unwrap();
    assert!(
        matches!(events.first(), Some(GenerationEvent::Started { id: actual }) if *actual == id)
    );
    assert!(events.iter().any(|event| matches!(event, GenerationEvent::Text { text, .. } if !text.is_empty() && text.len() <= 1024)));
    assert!(
        matches!(events.last(), Some(GenerationEvent::Finished { reason: GenerationEnd::Eos | GenerationEnd::Limit, stats, error: None, .. }) if stats.input_tokens > 0 && stats.output_tokens > 0 && stats.output_tokens <= 32 && stats.first_token_us.is_some() && stats.total_us > 0)
    );
    assert!(REGISTRY.lock().unwrap().active.is_none());
    handle.close().unwrap();
}

#[test]
#[ignore = "requires verified immutable internal encoder-only BGE and runtime RAM admission"]
fn real_encoder_mislabeled_generation_is_refused_before_context() {
    let _serial = delivery_serial();
    let path = std::env::var("TONALLI_TEST_BGE_INTERNAL").unwrap();
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Generation).unwrap();
    let sink = Arc::new(Recording(Mutex::new(Vec::new())));
    assert!(matches!(
        generate(handle.clone(), "Hi".into(), 8, sink.clone()),
        Err(PackError::Mismatch(_))
    ));
    assert!(sink.0.lock().unwrap().is_empty());
    assert!(REGISTRY.lock().unwrap().active.is_none());
    handle.close().unwrap();
}

#[test]
#[ignore = "requires verified BGE and runtime admission; creates context but performs no decode or embedding"]
fn real_encoder_native_context_is_noncausal_without_decoding() {
    use llama_cpp_2::context::params::LlamaContextParams;
    use std::num::NonZeroU32;
    let _serial = delivery_serial();
    let path = std::env::var("TONALLI_TEST_BGE_INTERNAL").unwrap();
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Generation).unwrap();
    let resources = handle
        .inner
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .resources
        .clone();
    let mut workspace = resources.workspace().unwrap();
    let context = resources
        .native
        .new_context(
            super::super::model::backend().unwrap(),
            LlamaContextParams::default()
                .with_n_ctx(NonZeroU32::new(512))
                .with_n_batch(64)
                .with_n_ubatch(64)
                .with_n_threads(1)
                .with_n_threads_batch(1)
                .with_embeddings(false)
                .with_offload_kqv(false),
        )
        .unwrap();
    workspace.loaded().unwrap();
    eprintln!(
        "bge_native_caps has_encoder={} has_decoder={} causal={}",
        resources.native.has_encoder(),
        resources.native.has_decoder(),
        context.is_causal()
    );
    assert!(
        !context.is_causal(),
        "noncausal context must be rejected even if an encoder GGUF carries a chat template"
    );
    assert!(REGISTRY.lock().unwrap().active.is_none());
    drop(context);
    drop(workspace);
    drop(resources);
    handle.close().unwrap();
}

#[test]
#[ignore = "requires verified Qwen, runtime RAM admission and genuine mid-native decode cancellation"]
fn real_prefill_and_token_native_abort_complete_within_one_second() {
    let _serial = delivery_serial();
    let path = std::env::var("TONALLI_TEST_QWEN_INTERNAL").unwrap();
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Generation).unwrap();
    for phase in [1, 2] {
        let mut qualified = false;
        for attempt in 0..6 {
            wait_for(|| !delivery::busy() && delivery::streams() == 0);
            let sink = Arc::new(Recording(Mutex::new(Vec::new())));
            let prompt = format!(
                "{} Continue a long numbered list. /no_think",
                "A short fact about offline libraries. ".repeat(100)
            );
            let id = generate(handle.clone(), prompt, 512, sink.clone()).unwrap();
            let job = REGISTRY.lock().unwrap().active.as_ref().unwrap().clone();
            let deadline = Instant::now();
            while job.decoding.load(Ordering::Acquire) != phase {
                assert!(deadline.elapsed() < std::time::Duration::from_secs(30));
                assert!(
                    job.result.lock().unwrap().is_none(),
                    "generation finished before selected native phase"
                );
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
            // No hook pauses native execution: this races a genuinely running decode.
            std::thread::sleep(std::time::Duration::from_millis(1));
            let before_cancel = Instant::now();
            cancel(id).unwrap();
            let result = job.result.lock().unwrap();
            let (result, timeout) = job
                .done
                .wait_timeout_while(result, std::time::Duration::from_secs(1), |result| {
                    result.is_none()
                })
                .unwrap();
            assert!(
                !timeout.timed_out() && result.is_some(),
                "native cleanup exceeded 1s"
            );
            assert!(result.as_ref().unwrap().is_ok());
            assert!(before_cancel.elapsed() < std::time::Duration::from_secs(1));
            drop(result);
            let cancellation = job.cancelled_at.lock().unwrap().unwrap();
            let traces = job.traces.lock().unwrap();
            qualified = traces.iter().any(|trace| {
                trace.phase == phase
                    && trace.before < cancellation
                    && cancellation < trace.after
                    && trace.code == 2
            });
            for trace in traces.iter().filter(|trace| {
                trace.phase == phase && trace.before < cancellation && cancellation < trace.after
            }) {
                eprintln!("native_interval phase={phase} before_us={} cancel_us={} after_us={} native_return={}", trace.before.duration_since(job.started).as_micros(), cancellation.duration_since(job.started).as_micros(), trace.after.duration_since(job.started).as_micros(), trace.code);
            }
            drop(traces);
            eprintln!(
                "native_abort phase={phase} attempt={} qualified={qualified} cleanup_us={}",
                attempt + 1,
                before_cancel.elapsed().as_micros()
            );
            wait_for(|| delivery::streams() == 0 && !delivery::busy());
            assert!(matches!(
                sink.0.lock().unwrap().last(),
                Some(GenerationEvent::Finished {
                    reason: GenerationEnd::Cancelled,
                    ..
                })
            ));
            if qualified {
                break;
            }
        }
        assert!(
            qualified,
            "no genuine mid-decode return2 observed in bounded attempts for phase {phase}"
        );
    }
    handle.close().unwrap();
}

#[test]
#[ignore = "requires verified Qwen and runtime RAM admission; deliberately stalls foreign sink"]
fn real_close_and_native_cleanup_do_not_wait_for_foreign_sink() {
    struct Blocked {
        gate: Arc<(Mutex<bool>, Condvar)>,
        entered: Arc<AtomicBool>,
    }
    impl TokenSink for Blocked {
        fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
            let guard = self.gate.0.lock().unwrap();
            self.entered.store(true, Ordering::Release);
            drop(self.gate.1.wait_while(guard, |open| !*open).unwrap());
            Ok(())
        }
    }
    let _serial = delivery_serial();
    let path = std::env::var("TONALLI_TEST_QWEN_INTERNAL").unwrap();
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Generation).unwrap();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let id = generate(
        handle.clone(),
        "Write a long list. /no_think".into(),
        512,
        Arc::new(Blocked {
            gate: gate.clone(),
            entered: entered.clone(),
        }),
    )
    .unwrap();
    let job = REGISTRY.lock().unwrap().active.as_ref().unwrap().clone();
    let start = Instant::now();
    let callback_entered_during_decode = loop {
        if entered.load(Ordering::Acquire)
            && job.decoding.load(Ordering::Acquire) != 0
            && delivery::busy()
        {
            break true;
        }
        if start.elapsed() >= std::time::Duration::from_secs(30) {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    };
    let info_responsive = handle.info().is_ok();
    let closed = Instant::now();
    handle.close().unwrap();
    let close_elapsed = closed.elapsed();
    let alias_invalidated = matches!(handle.info(), Err(PackError::NotFound(_)));
    cancel(id).unwrap();
    let result = job.result.lock().unwrap();
    let (result, timeout) = job
        .done
        .wait_timeout_while(result, std::time::Duration::from_secs(1), |result| {
            result.is_none()
        })
        .unwrap();
    let completed = !timeout.timed_out() && result.is_some();
    drop(result);
    let blocked_at_cleanup =
        entered.load(Ordering::Acquire) && delivery::busy() && !*gate.0.lock().unwrap();
    eprintln!("blocked_sink_proof entered_during_native_decode={callback_entered_during_decode} blocked_at_cleanup={blocked_at_cleanup} native_cleanup_complete={completed} close_us={}", close_elapsed.as_micros());
    // Release the test callback even if the cleanup assertion fails.
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert!(callback_entered_during_decode && blocked_at_cleanup);
    assert!(info_responsive && alias_invalidated);
    assert!(close_elapsed < std::time::Duration::from_millis(100));
    assert!(
        completed,
        "native resources must be released while foreign callback remains blocked"
    );
    assert!(REGISTRY.lock().unwrap().active.is_none());
    wait_for(|| delivery::streams() == 0 && !delivery::busy());
    assert_eq!(delivery::thread_count(), 1);
}

#[test]
fn native_capability_getters_exist_without_exposing_model_pointers() {
    fn contract(model: &llama_cpp_2::model::LlamaModel) {
        let _: bool = model.has_encoder();
        let _: bool = model.has_decoder();
    }
    let _compile_contract = contract;
}

#[test]
fn context_causal_capability_is_available_before_logits_or_sampling() {
    fn contract(context: &llama_cpp_2::context::LlamaContext<'_>) {
        let _: bool = context.is_causal();
    }
    let _compile_contract = contract;
}

#[test]
fn encoder_only_and_mislabeled_models_are_refused() {
    assert!(validate_request(ModelKind::Generation, true, false, "Hi", 8).is_err());
    assert!(validate_request(ModelKind::Generation, true, true, "Hi", 8).is_err());
    assert!(validate_request(ModelKind::Generation, false, false, "Hi", 8).is_err());
    assert!(validate_request(ModelKind::Embedding, false, true, "Hi", 8).is_err());
    validate_request(ModelKind::Generation, false, true, "Hola", 8).unwrap();
}

#[test]
fn raw_prompt_and_output_requests_have_explicit_bounds() {
    for prompt in [
        "".to_owned(),
        " \n ".to_owned(),
        "\0".to_owned(),
        "x".repeat(65_537),
    ] {
        assert!(validate_request(ModelKind::Generation, false, true, &prompt, 8).is_err());
    }
    for limit in [0, 1025, u32::MAX] {
        assert!(validate_request(ModelKind::Generation, false, true, "Hi", limit).is_err());
    }
}

#[test]
fn context_uses_actual_training_limit_and_checked_token_counts() {
    assert_eq!(token_budget(2000, 1024, 40_960).unwrap(), 4096);
    assert_eq!(token_budget(2000, 48, 2048).unwrap(), 2048);
    for (input, output, context) in [
        (2049, 1, 4096),
        (1000, 25, 1024),
        (1, 1, 0),
        (usize::MAX, u32::MAX, u32::MAX),
    ] {
        assert!(token_budget(input, output, context).is_err());
    }
}

#[test]
fn utf8_preserves_spanish_cjk_and_emoji_across_token_pieces() {
    let mut pending = Vec::new();
    assert_eq!(utf8(&mut pending, &[0xc3], false).unwrap(), "");
    assert_eq!(utf8(&mut pending, &[0xb1, 0xe4, 0xb8], false).unwrap(), "ñ");
    assert_eq!(
        utf8(&mut pending, &[0xad, 0xf0, 0x9f, 0x8c], false).unwrap(),
        "中"
    );
    assert_eq!(utf8(&mut pending, &[0x8e], false).unwrap(), "🌎");
    assert!(pending.is_empty());
}

#[test]
fn invalid_utf8_is_replaced_and_eos_flushes_only_the_remaining_tail() {
    let mut pending = Vec::new();
    assert_eq!(utf8(&mut pending, b"A\xffB\xe4", false).unwrap(), "A�B");
    assert_eq!(utf8(&mut pending, &[], true).unwrap(), "�");
    assert!(pending.is_empty());
    assert!(utf8(&mut pending, &vec![b'x'; 16_385], false).is_err());
}

#[test]
fn registry_is_bounded_and_stale_cancellation_cannot_target_a_new_job() {
    let mut registry = Registry::default();
    let first = registry.reserve().unwrap();
    assert!(registry.reserve().is_err());
    registry.cancel(first.id);
    assert!(first.flag.load(Ordering::Acquire));
    registry.complete(first.id);
    let second = registry.reserve().unwrap();
    assert!(second.id > first.id);
    registry.cancel(first.id);
    assert!(!second.flag.load(Ordering::Acquire));
    registry.complete(first.id);
    assert!(registry.active.is_some());
    registry.complete(second.id);
    assert!(registry.active.is_none());
}

#[test]
fn enqueue_cancellation_is_cancelled_but_recorded_sink_failure_stays_failed() {
    let job = Registry::default().reserve().unwrap();
    job.flag.store(true, Ordering::Release);
    let enqueue = delivery::text(job.clone(), "x".into());
    assert!(matches!(worker::delivery_result(&job, enqueue), Ok(false)));
    job.fail(PackError::Corrupt("actual callback failure".into()));
    let enqueue = delivery::text(job.clone(), "x".into());
    assert!(worker::delivery_result(&job, enqueue).is_err());
}

static DELIVERY_TEST_LOCK: Mutex<()> = Mutex::new(());

fn delivery_serial() -> std::sync::MutexGuard<'static, ()> {
    DELIVERY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn wait_for(condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn sink_errors_and_panics_become_typed_cancellation() {
    struct Failing(bool);
    impl TokenSink for Failing {
        fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
            if self.0 {
                panic!("test sink panic");
            }
            Err(PackError::Io("test sink error".into()))
        }
    }
    let _serial = delivery_serial();
    for panic in [false, true] {
        let job = Registry::default().reserve().unwrap();
        delivery::begin(Arc::clone(&job), Arc::new(Failing(panic))).unwrap();
        wait_for(|| job.flag.load(Ordering::Acquire));
        assert!(matches!(
            job.failure.lock().unwrap().as_ref(),
            Some(PackError::Corrupt(_))
        ));
        wait_for(|| !delivery::busy());
        delivery::finish(
            Arc::clone(&job),
            GenerationEvent::Finished {
                id: job.id,
                reason: GenerationEnd::Failed,
                stats: GenerationStats::default(),
                error: Some("test".into()),
            },
        );
        wait_for(|| delivery::streams() == 0);
    }
    assert_eq!(delivery::thread_count(), 1);
}

#[test]
fn blocked_sink_backpressure_is_bounded_and_never_spawns_replacement_dispatchers() {
    use std::sync::atomic::AtomicUsize;
    struct Blocking {
        gate: Arc<(Mutex<bool>, Condvar)>,
        entered: Arc<AtomicBool>,
        terminals: Arc<AtomicUsize>,
    }
    impl TokenSink for Blocking {
        fn on_event(&self, event: GenerationEvent) -> Result<(), PackError> {
            if matches!(event, GenerationEvent::Finished { .. }) {
                self.terminals.fetch_add(1, Ordering::AcqRel);
            }
            self.entered.store(true, Ordering::Release);
            let (lock, wake) = &*self.gate;
            let guard = lock.lock().unwrap();
            drop(wake.wait_while(guard, |ready| !*ready).unwrap());
            Ok(())
        }
    }
    let _serial = delivery_serial();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let terminals = Arc::new(AtomicUsize::new(0));
    let job = Registry::default().reserve().unwrap();
    delivery::begin(
        Arc::clone(&job),
        Arc::new(Blocking {
            gate: Arc::clone(&gate),
            entered: Arc::clone(&entered),
            terminals: Arc::clone(&terminals),
        }),
    )
    .unwrap();
    wait_for(|| entered.load(Ordering::Acquire));
    assert!(delivery::begin(
        Arc::clone(&job),
        Arc::new(Blocking {
            gate: Arc::clone(&gate),
            entered,
            terminals: Arc::clone(&terminals)
        })
    )
    .is_err());
    let start = Instant::now();
    let mut full = false;
    for _ in 0..40 {
        if delivery::text(Arc::clone(&job), "x".into()).is_err() {
            full = true;
            break;
        }
    }
    assert!(full);
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    assert!(job.flag.load(Ordering::Acquire));
    assert_eq!(delivery::thread_count(), 1);
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    delivery::finish(
        Arc::clone(&job),
        GenerationEvent::Finished {
            id: job.id,
            reason: GenerationEnd::Failed,
            stats: GenerationStats::default(),
            error: Some("test".into()),
        },
    );
    wait_for(|| delivery::streams() == 0);
    assert_eq!(
        terminals.load(Ordering::Acquire),
        1,
        "recovering sink must receive its failed terminal once"
    );
}

#[test]
fn terminal_requires_accepted_begin_and_is_idempotent() {
    struct Sink;
    impl TokenSink for Sink {
        fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
            Ok(())
        }
    }
    let _serial = delivery_serial();
    let job = Registry::default().reserve().unwrap();
    delivery::begin(Arc::clone(&job), Arc::new(Sink)).unwrap();
    let event = GenerationEvent::Finished {
        id: job.id,
        reason: GenerationEnd::Limit,
        stats: GenerationStats::default(),
        error: None,
    };
    delivery::finish(Arc::clone(&job), event.clone());
    delivery::finish(Arc::clone(&job), event.clone());
    let unaccepted = Registry::default().reserve().unwrap();
    delivery::finish(unaccepted, event);
    delivery::flush();
    assert_eq!(delivery::streams(), 0);
}
