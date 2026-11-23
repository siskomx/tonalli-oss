use super::*;
use llama_cpp_2::context::LlamaContext;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct NativeTrace {
    before: Instant,
    after: Instant,
    code: i32,
}

static DECODING: AtomicBool = AtomicBool::new(false);
static TRACES: Mutex<Vec<NativeTrace>> = Mutex::new(Vec::new());
static REAL_SERIAL: Mutex<()> = Mutex::new(());

pub(super) fn before_decode() -> Instant {
    DECODING.store(true, Ordering::Release);
    Instant::now()
}

pub(super) fn after_decode(before: Instant, outcome: &Result<(), DecodeError>) {
    let after = Instant::now();
    DECODING.store(false, Ordering::Release);
    let code = match outcome {
        Ok(()) => 0,
        Err(DecodeError::Unknown(code)) => *code,
        _ => -1,
    };
    if let Ok(mut traces) = TRACES.lock() {
        if traces.len() < 21 {
            traces.push(NativeTrace {
                before,
                after,
                code,
            });
        }
    }
}

#[test]
fn pooling_getter_is_a_safe_read_only_context_api() {
    let _getter = |context: &LlamaContext<'_>| -> LlamaPoolingType { context.pooling_type() };
}

#[test]
fn inputs_have_count_utf8_byte_and_aggregate_bounds() {
    input_bounds(&vec!["text".into(); 21]).unwrap();
    input_bounds(&["a".repeat(16_384)]).unwrap();
    input_bounds(&vec!["a".repeat(16_384); 16]).unwrap();
    for texts in [
        vec![],
        vec!["text".into(); 22],
        vec!["".into()],
        vec![" \n\t".into()],
        vec!["a\0b".into()],
        vec!["a".repeat(16_385)],
        vec!["🌎".repeat(4097)],
        vec!["a".repeat(16_384); 17],
    ] {
        assert!(matches!(input_bounds(&texts), Err(PackError::Mismatch(_))));
    }
}

#[test]
fn closed_handle_returns_a_typed_error_without_native_work() {
    assert!(matches!(
        embed(AiHandle::closed_for_test(), vec!["text".into()]),
        Err(PackError::NotFound(_))
    ));
}

#[test]
fn generation_hint_and_invalid_native_dimensions_are_rejected() {
    assert_eq!(model_bounds(ModelKind::Embedding, 384).unwrap(), 384);
    assert_eq!(model_bounds(ModelKind::Embedding, 4096).unwrap(), 4096);
    assert!(matches!(
        model_bounds(ModelKind::Generation, 384),
        Err(PackError::Mismatch(_))
    ));
    for width in [0, 4097, u32::MAX] {
        assert!(matches!(
            model_bounds(ModelKind::Embedding, width),
            Err(PackError::Corrupt(_))
        ));
    }
    assert_eq!(
        21 * model_bounds(ModelKind::Embedding, 4096).unwrap() * 4,
        344_064
    );
}

#[test]
fn actual_tokens_fit_one_bounded_physical_batch() {
    assert_eq!(token_budget(512, 512).unwrap(), 512);
    assert_eq!(token_budget(256, 256).unwrap(), 256);
    assert_eq!(token_budget(1, 40960).unwrap(), 512);
    for (tokens, training) in [(0, 512), (513, 512), (257, 256), (usize::MAX, 512)] {
        assert!(matches!(
            token_budget(tokens, training),
            Err(PackError::Mismatch(_))
        ));
    }
    assert!(matches!(token_budget(1, 0), Err(PackError::Corrupt(_))));
}

#[test]
fn native_context_requires_noncausal_vector_pooling() {
    for pooling in [
        LlamaPoolingType::Mean,
        LlamaPoolingType::Cls,
        LlamaPoolingType::Last,
    ] {
        pooling_bounds(false, pooling).unwrap();
        assert!(matches!(
            pooling_bounds(true, pooling),
            Err(PackError::Mismatch(_))
        ));
    }
    for pooling in [
        LlamaPoolingType::Unspecified,
        LlamaPoolingType::None,
        LlamaPoolingType::Rank,
    ] {
        assert!(matches!(
            pooling_bounds(false, pooling),
            Err(PackError::Mismatch(_))
        ));
    }
}

#[test]
fn finite_vectors_are_normalized_without_float32_square_overflow() {
    let vector = normalized(&[3.0, 4.0], 2).unwrap();
    assert!((vector[0] - 0.6).abs() < 0.00001);
    assert!((vector[1] - 0.8).abs() < 0.00001);
    let large = normalized(&[f32::MAX, f32::MAX], 2).unwrap();
    assert!(large.iter().all(|value| value.is_finite()));
    assert!(
        (large
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            - 1.0)
            .abs()
            < 0.00001
    );
    for values in [
        vec![],
        vec![0.0, 0.0],
        vec![f32::NAN, 1.0],
        vec![f32::INFINITY, 1.0],
        vec![1.0],
    ] {
        assert!(matches!(normalized(&values, 2), Err(PackError::Corrupt(_))));
    }
}

#[test]
fn busy_admission_preserves_the_original_abort_owner() {
    let mut slot = None;
    let first = claim_abort(&mut slot).unwrap();
    assert!(matches!(
        claim_abort(&mut slot),
        Err(PackError::Mismatch(_))
    ));
    assert!(Arc::ptr_eq(
        &first,
        &slot.as_ref().unwrap().upgrade().unwrap()
    ));
    first.store(true, Ordering::Release);
    assert!(matches!(
        claim_abort(&mut slot),
        Err(PackError::Mismatch(_))
    ));
    drop(first);
    let next = claim_abort(&mut slot).unwrap();
    assert!(!next.load(Ordering::Acquire));
    assert!(Arc::ptr_eq(&next, &slot.unwrap().upgrade().unwrap()));
}

#[test]
#[ignore = "requires verified internal BGE and a separate guarded real-model lease"]
fn real_bge_vectors_are_finite_normalized_and_independent() {
    let _serial = REAL_SERIAL.lock().unwrap();
    let path = std::env::var("TONALLI_TEST_BGE_INTERNAL").expect("verified internal BGE path");
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Embedding).unwrap();
    assert_eq!(handle.info().unwrap().embedding_dimensions, 384);
    let first = "A red fox crosses a snowy field.".to_owned();
    let vectors = embed(
        handle.clone(),
        vec![
            first.clone(),
            "Mitochondria produce cellular energy.".into(),
            first,
        ],
    )
    .unwrap();
    assert_eq!(vectors.len(), 3);
    for vector in &vectors {
        assert_eq!(vector.len(), 384);
        assert!(vector.iter().all(|value| value.is_finite()));
        assert!(
            (vector
                .iter()
                .map(|value| f64::from(*value).powi(2))
                .sum::<f64>()
                - 1.0)
                .abs()
                < 0.00001
        );
    }
    assert!(vectors[0]
        .iter()
        .zip(&vectors[2])
        .all(|(left, right)| (left - right).abs() < 0.00001));
    assert!(vectors[0]
        .iter()
        .zip(&vectors[1])
        .any(|(left, right)| (left - right).abs() > 0.0001));
    handle.close().unwrap();
}

#[test]
#[ignore = "requires verified internal Qwen and a separate guarded real-model lease"]
fn real_causal_qwen_mislabeled_embedding_is_refused() {
    let _serial = REAL_SERIAL.lock().unwrap();
    let path = std::env::var("TONALLI_TEST_QWEN_INTERNAL").expect("verified internal Qwen path");
    let handle =
        super::super::load_internal(std::path::Path::new(&path), ModelKind::Embedding).unwrap();
    assert!(
        matches!(embed(handle.clone(), vec!["One bounded test sentence.".into()]),
        Err(PackError::Mismatch(message)) if message.contains("noncausal"))
    );
    handle.close().unwrap();
}

#[test]
#[ignore = "requires verified BGE, separate memory guard and an observed actual native return-2 abort"]
fn real_close_cancels_native_embedding_without_blocking_info() {
    let _serial = REAL_SERIAL.lock().unwrap();
    let path = std::env::var("TONALLI_TEST_BGE_INTERNAL").expect("verified internal BGE path");
    for attempt in 1..=8 {
        let handle =
            super::super::load_internal(std::path::Path::new(&path), ModelKind::Embedding).unwrap();
        let text = "a ".repeat(500);
        {
            let open = handle.inner.lock().unwrap();
            let native = &open.as_ref().unwrap().resources.native;
            let count = native.vocab().tokenize(text.as_bytes(), true, false).len();
            assert!(
                (400..=512).contains(&count),
                "real BGE probe token count {count}"
            );
        }
        DECODING.store(false, Ordering::Release);
        TRACES.lock().unwrap().clear();
        let worker_handle = handle.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = embed(worker_handle, vec![text; 21]);
            let _ = sender.send((Instant::now(), result));
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !DECODING.load(Ordering::Acquire)
            && !worker.is_finished()
            && Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        let observed = DECODING.load(Ordering::Acquire);
        let info_start = Instant::now();
        let info = handle.info();
        let info_us = info_start.elapsed().as_micros();
        let cancelled_at = Instant::now();
        handle.close().unwrap();
        let close_us = cancelled_at.elapsed().as_micros();
        let (finished_at, result) = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("embedding must return after close");
        worker.join().unwrap();
        assert_eq!(info.unwrap().embedding_dimensions, 384);
        assert!(info_us < 1_000_000 && close_us < 1_000_000);
        assert!(matches!(handle.info(), Err(PackError::NotFound(_))));
        match &result {
            Err(PackError::Mismatch(message)) if message.contains("cancelled") => {}
            Err(error) => panic!("embedding failed before qualified cancellation: {error:?}"),
            Ok(_) => {} // Completion can legitimately win the race; this attempt proves no abort.
        }
        let genuine_abort = TRACES.lock().unwrap().iter().any(|trace| {
            trace.code == 2 && trace.before <= cancelled_at && cancelled_at <= trace.after
        });
        if genuine_abort {
            assert!(
                matches!(result, Err(PackError::Mismatch(message)) if message.contains("cancelled"))
            );
            assert!(finished_at.duration_since(cancelled_at) <= Duration::from_secs(1));
            eprintln!("embedding_close_proof attempt={attempt} observed_native_phase={observed} native_return2=true info_us={info_us} close_us={close_us} cleanup_us={}",
                finished_at.duration_since(cancelled_at).as_micros());
            return;
        }
    }
    panic!("inconclusive: no qualified native return-2 cancellation observed in eight bounded attempts");
}
