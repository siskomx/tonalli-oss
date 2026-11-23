//! Per-answer owned event collection on the existing generation dispatcher.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::Instant,
};

use crate::{
    ai::{GenId, GenerationEvent, TokenSink},
    pack::PackError,
};

#[derive(Clone, Debug)]
pub(super) struct Collected {
    pub id: Option<GenId>,
    pub raw_text: String,
    pub terminal: Option<GenerationEvent>,
    pub terminal_forwarded: bool,
    pub failure: Option<PackError>,
    pub deadline: bool,
    /// The calling worker acts on this only after wait returns without a state lock.
    pub cancel_id: Option<GenId>,
    pub first_raw_received_us: Option<u64>,
}

pub(super) struct Collector {
    foreign: Arc<dyn TokenSink>,
    state: Mutex<State>,
    changed: Condvar,
    started: Instant,
}

struct State {
    result: Collected,
    started: bool,
    returned: Option<GenId>,
    chunks: usize,
    forwarding: bool,
}

impl State {
    fn fail(&mut self, message: &str) -> PackError {
        let error = PackError::Corrupt(format!("AI answer collector: {message}"));
        if self.result.failure.is_none() {
            self.result.failure = Some(error.clone());
        }
        self.result.cancel_id = self.returned.or(self.result.id);
        self.forwarding = false;
        error
    }
}

impl Collector {
    pub fn new(foreign: Arc<dyn TokenSink>, started: Instant) -> Self {
        Self {
            foreign,
            state: Mutex::new(State {
                result: Collected {
                    id: None,
                    raw_text: String::new(),
                    terminal: None,
                    terminal_forwarded: false,
                    failure: None,
                    deadline: false,
                    cancel_id: None,
                    first_raw_received_us: None,
                },
                started: false,
                returned: None,
                chunks: 0,
                forwarding: true,
            }),
            changed: Condvar::new(),
            started,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.fail("state lock poisoned");
                state
            }
        }
    }

    pub fn returned(&self, id: GenId) -> Result<(), PackError> {
        let mut state = self.lock();
        let mismatch = id == 0 || state.result.id.is_some_and(|previous| previous != id);
        if id != 0 {
            state.returned = Some(id);
        }
        if mismatch {
            let error = state.fail("returned generation ID mismatch");
            self.changed.notify_all();
            return Err(error);
        }
        state.result.id = Some(id);
        if !state.forwarding {
            state.result.cancel_id = Some(id);
        }
        Ok(())
    }

    pub fn wait(&self, deadline: Instant) -> Collected {
        let mut state = self.lock();
        loop {
            if state.result.failure.is_some() || state.result.terminal_forwarded {
                return state.result.clone();
            }
            let now = Instant::now();
            if state.result.deadline || now >= deadline {
                state.result.deadline = true;
                state.result.cancel_id = state.returned.or(state.result.id);
                state.forwarding = false;
                return state.result.clone();
            }
            state = match self
                .changed
                .wait_timeout(state, deadline.saturating_duration_since(now))
            {
                Ok((state, _)) => state,
                Err(poisoned) => {
                    let (mut state, _) = poisoned.into_inner();
                    state.fail("wait lock poisoned");
                    state
                }
            };
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> Collected {
        self.lock().result.clone()
    }
}

impl TokenSink for Collector {
    fn on_event(&self, event: GenerationEvent) -> Result<(), PackError> {
        let received = Instant::now();
        let terminal = matches!(event, GenerationEvent::Finished { .. });
        {
            let mut state = self.lock();
            if !state.forwarding {
                return Ok(());
            }
            let valid = match &event {
                GenerationEvent::Started { id } => {
                    *id != 0
                        && !state.started
                        && state.result.id.is_none_or(|previous| previous == *id)
                }
                GenerationEvent::Text { id, text } => {
                    state.started
                        && state.result.id == Some(*id)
                        && state.result.terminal.is_none()
                        && !text.is_empty()
                        && text.len() <= 1024
                        && state.chunks < 4096
                        && state
                            .result
                            .raw_text
                            .len()
                            .checked_add(text.len())
                            .is_some_and(|bytes| bytes <= 128 * 1024)
                }
                GenerationEvent::Finished { id, error, .. } => {
                    state.started
                        && state.result.id == Some(*id)
                        && state.result.terminal.is_none()
                        && error.as_ref().is_none_or(|error| error.len() <= 4096)
                }
            };
            if !valid {
                let error = state.fail("invalid event ordering, ID or bounds");
                self.changed.notify_all();
                return Err(error);
            }
            match &event {
                GenerationEvent::Started { id } => {
                    state.result.id = Some(*id);
                    state.started = true;
                }
                GenerationEvent::Text { text, .. } => {
                    if state.result.first_raw_received_us.is_none() {
                        state.result.first_raw_received_us = Some(
                            u64::try_from(
                                received.saturating_duration_since(self.started).as_micros(),
                            )
                            .unwrap_or(u64::MAX),
                        );
                    }
                    state.result.raw_text.push_str(text);
                    state.chunks += 1;
                }
                GenerationEvent::Finished { .. } => state.result.terminal = Some(event.clone()),
            }
            self.changed.notify_all();
        }
        // The existing dispatcher owns this call; no state lock or native resources cross it.
        let forwarded = catch_unwind(AssertUnwindSafe(|| self.foreign.on_event(event)));
        let mut state = self.lock();
        match forwarded {
            Ok(Ok(())) => {
                if terminal {
                    state.result.terminal_forwarded = true;
                }
            }
            _ => {
                let error = state.fail("foreign callback failed or panicked");
                self.changed.notify_all();
                return Err(error);
            }
        }
        self.changed.notify_all();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
