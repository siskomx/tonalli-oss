use super::*;
use crate::ai::{GenerationEnd, GenerationStats};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Mutex, Weak,
};
use std::{thread, time::Duration};

fn new_collector(sink: Arc<dyn TokenSink>) -> Collector {
    Collector::new(sink, Instant::now())
}

#[derive(Default)]
struct Count(AtomicUsize);
impl TokenSink for Count {
    fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn started(id: GenId) -> GenerationEvent {
    GenerationEvent::Started { id }
}
fn text(id: GenId, text: &str) -> GenerationEvent {
    GenerationEvent::Text {
        id,
        text: text.into(),
    }
}
fn terminal(id: GenId) -> GenerationEvent {
    GenerationEvent::Finished {
        id,
        reason: GenerationEnd::Eos,
        stats: GenerationStats::default(),
        error: None,
    }
}

#[test]
fn early_started_matches_returned_id_and_terminal_is_owned() {
    let sink = Arc::new(Count::default());
    let collector = new_collector(sink.clone());
    collector.on_event(started(41)).unwrap();
    collector.on_event(text(41, "Hola [1]")).unwrap();
    collector.returned(41).unwrap();
    collector.on_event(terminal(41)).unwrap();
    let result = collector.wait(Instant::now());
    assert_eq!(result.id, Some(41));
    assert_eq!(result.raw_text, "Hola [1]");
    assert!(matches!(
        result.terminal,
        Some(GenerationEvent::Finished { id: 41, .. })
    ));
    assert!(result.terminal_forwarded && !result.deadline && result.failure.is_none());
    assert_eq!(sink.0.load(Ordering::SeqCst), 3);
}

#[test]
fn first_raw_receipt_is_owned_missing_before_text_and_not_replaced_by_later_text() {
    let before = Instant::now();
    let collector = new_collector(Arc::new(Count::default()));
    collector.on_event(started(41)).unwrap();
    assert!(collector.snapshot().first_raw_received_us.is_none());
    collector.on_event(text(41, "first")).unwrap();
    let first = collector
        .snapshot()
        .first_raw_received_us
        .expect("actual first Text receipt");
    assert!(u128::from(first) <= before.elapsed().as_micros());
    collector.on_event(text(41, "later")).unwrap();
    let timed_out = collector.wait(Instant::now());
    assert_eq!(timed_out.first_raw_received_us, Some(first));
    assert!(timed_out.deadline && timed_out.terminal.is_none());
}

#[test]
fn wrong_ids_duplicate_terminals_and_missing_started_are_refused() {
    let collector = new_collector(Arc::new(Count::default()));
    collector.returned(41).unwrap();
    assert!(collector.on_event(started(42)).is_err());
    assert_eq!(collector.wait(Instant::now()).cancel_id, Some(41));
    let collector = new_collector(Arc::new(Count::default()));
    collector.on_event(started(41)).unwrap();
    assert!(collector.returned(42).is_err());
    assert_eq!(collector.wait(Instant::now()).cancel_id, Some(42));
    let collector = new_collector(Arc::new(Count::default()));
    assert!(collector.on_event(text(41, "not started")).is_err());
    let collector = new_collector(Arc::new(Count::default()));
    collector.on_event(started(41)).unwrap();
    collector.on_event(terminal(41)).unwrap();
    assert!(collector.on_event(terminal(41)).is_err());
    assert!(collector.returned(0).is_err());
}

struct Inspect {
    owner: Mutex<Weak<Collector>>,
    observed: mpsc::Sender<Collected>,
}
impl TokenSink for Inspect {
    fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
        let collector = self.owner.lock().unwrap().upgrade().unwrap();
        self.observed.send(collector.snapshot()).unwrap();
        Ok(())
    }
}

#[test]
fn foreign_callback_observes_recorded_text_without_a_state_lock() {
    let (send, receive) = mpsc::channel();
    let sink = Arc::new(Inspect {
        owner: Mutex::new(Weak::new()),
        observed: send,
    });
    let collector = Arc::new(new_collector(sink.clone()));
    *sink.owner.lock().unwrap() = Arc::downgrade(&collector);
    let owner = collector.clone();
    let producer = thread::spawn(move || {
        owner.on_event(started(41)).unwrap();
        owner.on_event(text(41, "recorded")).unwrap();
    });
    assert_eq!(
        receive.recv_timeout(Duration::from_secs(2)).unwrap().id,
        Some(41)
    );
    assert_eq!(
        receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .raw_text,
        "recorded"
    );
    producer.join().unwrap();
}

struct Block {
    terminal_only: bool,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    count: AtomicUsize,
}
impl TokenSink for Block {
    fn on_event(&self, event: GenerationEvent) -> Result<(), PackError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        if matches!(event, GenerationEvent::Finished { .. }) == self.terminal_only
            && !matches!(event, GenerationEvent::Started { .. })
        {
            self.entered.send(()).unwrap();
            let _ = self.release.lock().unwrap().recv();
        }
        Ok(())
    }
}

#[test]
fn deadline_preserves_partial_and_disables_later_forwarding_without_terminal_fiction() {
    let (entered, receive) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let sink = Arc::new(Block {
        terminal_only: false,
        entered,
        release: Mutex::new(released),
        count: AtomicUsize::new(0),
    });
    let collector = Arc::new(new_collector(sink.clone()));
    collector.on_event(started(41)).unwrap();
    let owner = collector.clone();
    let producer = thread::spawn(move || owner.on_event(text(41, "partial")));
    receive.recv_timeout(Duration::from_secs(2)).unwrap();
    let result = collector.wait(Instant::now());
    assert!(result.deadline && result.terminal.is_none() && !result.terminal_forwarded);
    assert_eq!(result.raw_text, "partial");
    assert_eq!(result.cancel_id, Some(41));
    release.send(()).unwrap();
    producer.join().unwrap().unwrap();
    collector.on_event(text(41, "late")).unwrap();
    collector.on_event(terminal(41)).unwrap();
    assert_eq!(collector.snapshot().raw_text, "partial");
    assert_eq!(sink.count.load(Ordering::SeqCst), 2);
}

#[test]
fn observed_terminal_is_distinct_from_foreign_callback_completion() {
    let (entered, receive) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let sink = Arc::new(Block {
        terminal_only: true,
        entered,
        release: Mutex::new(released),
        count: AtomicUsize::new(0),
    });
    let collector = Arc::new(new_collector(sink));
    collector.on_event(started(41)).unwrap();
    let owner = collector.clone();
    let producer = thread::spawn(move || owner.on_event(terminal(41)));
    receive.recv_timeout(Duration::from_secs(2)).unwrap();
    let result = collector.wait(Instant::now());
    assert!(result.terminal.is_some() && !result.terminal_forwarded && result.deadline);
    release.send(()).unwrap();
    producer.join().unwrap().unwrap();
}

#[test]
fn text_chunk_aggregate_and_count_bounds_are_enforced_before_forwarding() {
    for (chunk, count) in [
        ("x".repeat(1025), 1),
        ("x".repeat(1024), 129),
        ("x".into(), 4097),
    ] {
        let collector = new_collector(Arc::new(Count::default()));
        collector.on_event(started(41)).unwrap();
        for _ in 1..count {
            collector.on_event(text(41, &chunk)).unwrap();
        }
        assert!(collector.on_event(text(41, &chunk)).is_err());
        let result = collector.wait(Instant::now());
        assert!(matches!(result.failure, Some(PackError::Corrupt(_))));
        assert_eq!(result.cancel_id, Some(41));
        assert!(result.raw_text.len() <= 128 * 1024);
    }
}

struct Fail(bool);
impl TokenSink for Fail {
    fn on_event(&self, _: GenerationEvent) -> Result<(), PackError> {
        if self.0 {
            panic!("synthetic callback panic");
        }
        Err(PackError::Io("synthetic callback error".into()))
    }
}
#[test]
fn foreign_error_and_panic_become_owned_failure_and_cancellation_request() {
    for panic in [false, true] {
        let collector = new_collector(Arc::new(Fail(panic)));
        assert!(collector.on_event(started(41)).is_err());
        let result = collector.wait(Instant::now());
        assert!(matches!(result.failure, Some(PackError::Corrupt(_))));
        assert_eq!(result.cancel_id, Some(41));
        assert!(!result.deadline && result.terminal.is_none());
    }
}
