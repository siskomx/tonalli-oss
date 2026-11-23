use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::{GenerationEvent, Job, TokenSink};
use crate::pack::PackError;

enum Message {
    Begin(Arc<Job>, Arc<dyn TokenSink>),
    Text(Arc<Job>, String),
    End(Arc<Job>, GenerationEvent),
    #[cfg(test)]
    Barrier(SyncSender<()>),
}

static SENDER: OnceLock<Result<SyncSender<Message>, PackError>> = OnceLock::new();
static BUSY: AtomicBool = AtomicBool::new(false);
static STREAMS: AtomicUsize = AtomicUsize::new(0);
static TEXTS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static CREATED: AtomicUsize = AtomicUsize::new(0);

fn sender() -> Result<&'static SyncSender<Message>, PackError> {
    SENDER
        .get_or_init(|| {
            // At most two delivery streams, each with Begin/End, plus 32 text slots.
            // Reserved control space makes terminal publication independent of backpressure.
            let (send, receive) = sync_channel(36);
            std::thread::Builder::new()
                .name("ai-token-sink".into())
                .spawn(move || {
                    let mut current: Option<(u64, Arc<dyn TokenSink>)> = None;
                    while let Ok(message) = receive.recv() {
                        BUSY.store(true, Ordering::Release);
                        match message {
                            Message::Begin(job, sink) => {
                                current = Some((job.id, sink));
                                if let Some((_, sink)) = &current {
                                    invoke(&job, sink, GenerationEvent::Started { id: job.id });
                                }
                            }
                            Message::Text(job, text) => {
                                TEXTS.fetch_sub(1, Ordering::AcqRel);
                                if !job.flag.load(Ordering::Acquire) {
                                    if let Some((id, sink)) = &current {
                                        if *id == job.id {
                                            invoke(
                                                &job,
                                                sink,
                                                GenerationEvent::Text { id: job.id, text },
                                            );
                                        }
                                    }
                                }
                            }
                            Message::End(job, event) => {
                                if !job.sink_broken.load(Ordering::Acquire) {
                                    if let Some((id, sink)) = &current {
                                        if *id == job.id {
                                            invoke(&job, sink, event);
                                        }
                                    }
                                }
                                current.take(); // Foreign sink ownership ends on this one dispatcher.
                                STREAMS.fetch_sub(1, Ordering::AcqRel);
                            }
                            #[cfg(test)]
                            Message::Barrier(reply) => {
                                let _ = reply.send(());
                            }
                        }
                        BUSY.store(false, Ordering::Release);
                    }
                })
                .map_err(PackError::from)?;
            #[cfg(test)]
            CREATED.fetch_add(1, Ordering::AcqRel);
            Ok(send)
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn invoke(job: &Job, sink: &Arc<dyn TokenSink>, event: GenerationEvent) {
    match catch_unwind(AssertUnwindSafe(|| sink.on_event(event))) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            job.sink_broken.store(true, Ordering::Release);
            job.fail(PackError::Corrupt(format!("AI token sink failed: {error}")));
        }
        Err(_) => {
            job.sink_broken.store(true, Ordering::Release);
            job.fail(PackError::Corrupt("AI token sink panicked".into()));
        }
    }
}

pub(super) fn begin(job: Arc<Job>, sink: Arc<dyn TokenSink>) -> Result<(), PackError> {
    if busy() {
        return Err(PackError::Mismatch("AI callback dispatcher is busy".into()));
    }
    let sender = sender()?;
    if job
        .delivery_started
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(PackError::Corrupt("AI delivery already started".into()));
    }
    if STREAMS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            if count < 2 {
                Some(count + 1)
            } else {
                None
            }
        })
        .is_err()
    {
        job.delivery_started.store(false, Ordering::Release);
        return Err(PackError::Mismatch("AI delivery capacity exhausted".into()));
    }
    if sender
        .try_send(Message::Begin(Arc::clone(&job), sink))
        .is_err()
    {
        STREAMS.fetch_sub(1, Ordering::AcqRel);
        job.delivery_started.store(false, Ordering::Release);
        return Err(PackError::Corrupt("AI callback dispatcher stopped".into()));
    }
    Ok(())
}

pub(super) fn text(job: Arc<Job>, text: String) -> Result<(), PackError> {
    if text.len() > 1024 {
        return Err(PackError::Corrupt("oversized AI text chunk".into()));
    }
    let sender = sender()?;
    let started = Instant::now();
    let mut message = Message::Text(Arc::clone(&job), text);
    loop {
        if job.flag.load(Ordering::Acquire) {
            return Err(PackError::Mismatch("AI generation cancelled".into()));
        }
        if TEXTS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                if count < 32 {
                    Some(count + 1)
                } else {
                    None
                }
            })
            .is_ok()
        {
            match sender.try_send(message) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    TEXTS.fetch_sub(1, Ordering::AcqRel);
                    message = returned;
                }
                Err(TrySendError::Disconnected(_)) => {
                    TEXTS.fetch_sub(1, Ordering::AcqRel);
                    let error = PackError::Corrupt("AI token dispatcher stopped".into());
                    job.fail(error.clone());
                    return Err(error);
                }
            }
        }
        if started.elapsed() >= Duration::from_millis(100) {
            let error = PackError::Corrupt("AI token sink backpressure".into());
            job.fail(error.clone());
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn finish(job: Arc<Job>, event: GenerationEvent) {
    if !job.delivery_started.load(Ordering::Acquire)
        || job
            .delivery_ended
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }
    // A terminal slot is reserved for each accepted Begin. No foreign callback here.
    if sender()
        .and_then(|sender| {
            sender
                .try_send(Message::End(Arc::clone(&job), event))
                .map_err(|_| PackError::Corrupt("AI terminal delivery stopped".into()))
        })
        .is_err()
    {
        job.fail(PackError::Corrupt("AI terminal delivery stopped".into()));
        STREAMS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) fn busy() -> bool {
    BUSY.load(Ordering::Acquire)
}

#[cfg(test)]
pub(super) fn thread_count() -> usize {
    CREATED.load(Ordering::Acquire)
}

#[cfg(test)]
pub(super) fn streams() -> usize {
    STREAMS.load(Ordering::Acquire)
}

#[cfg(test)]
pub(super) fn flush() {
    let (send, receive) = sync_channel(0);
    sender().unwrap().send(Message::Barrier(send)).unwrap();
    receive.recv_timeout(Duration::from_secs(2)).unwrap();
}
