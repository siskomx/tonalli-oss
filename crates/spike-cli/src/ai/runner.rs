use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tonalli_core::ai::{self, AiHandle, GenerationEnd, GenerationEvent, ModelKind, TokenSink};
use tonalli_core::pack::PackError;

use super::measurement::{assess, calibration_next, Collector, Request, Snapshot};
use super::model::VerifiedModel;
use super::Config;
use crate::manifest::{manifest_dir, write_atomic, Manifest};

fn invalid(message: &str) -> PackError {
    PackError::Corrupt(format!("AI benchmark: {message}"))
}
fn elapsed(start: Instant) -> Result<u64, PackError> {
    u64::try_from(start.elapsed().as_micros()).map_err(|_| invalid("clock overflow"))
}
fn cell(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

struct SinkState {
    collector: Collector,
    failure: Option<PackError>,
}
struct Sink {
    started: Instant,
    state: Mutex<SinkState>,
    wake: Condvar,
}
impl TokenSink for Sink {
    fn on_event(&self, event: GenerationEvent) -> Result<(), PackError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("sink lock poisoned"))?;
        let result = elapsed(self.started).and_then(|time| state.collector.observe(event, time));
        if let Err(error) = &result {
            state.failure = Some(error.clone());
        }
        if state.collector.request.is_some() || state.failure.is_some() {
            self.wake.notify_one();
        }
        result
    }
}

struct Runner {
    model: Arc<AiHandle>,
    stop: Arc<AtomicBool>,
    epoch: Instant,
    requests: Vec<Request>,
    snapshots: Vec<Snapshot>,
    next_snapshot_us: u64,
    request_csv: File,
    snapshot_csv: File,
}

impl Runner {
    fn snapshots(&mut self, now: u64) -> Result<(), PackError> {
        while now >= self.next_snapshot_us {
            if self.snapshots.len() >= 32 {
                return Err(invalid("snapshot bound exceeded"));
            }
            let latest = self.requests.last();
            let snapshot = Snapshot {
                scheduled_us: self.next_snapshot_us,
                observed_us: now,
                latest_request_id: latest.map(|request| request.id),
            };
            writeln!(
                self.snapshot_csv,
                "{},{},{},{},{},{},{},{}",
                snapshot.scheduled_us,
                now,
                latest.map(|r| r.id.to_string()).unwrap_or_default(),
                latest.map(|r| r.start_us.to_string()).unwrap_or_default(),
                latest.map(|r| r.end_us.to_string()).unwrap_or_default(),
                latest
                    .map(|r| r.stats.output_tokens.to_string())
                    .unwrap_or_default(),
                latest
                    .and_then(Request::wall_us)
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                latest
                    .and_then(Request::wall_tokens_s)
                    .map(|v| v.to_string())
                    .unwrap_or_default()
            )?;
            self.snapshot_csv.flush()?;
            self.snapshots.push(snapshot);
            self.next_snapshot_us = self
                .next_snapshot_us
                .checked_add(30_000_000)
                .ok_or_else(|| invalid("snapshot clock overflow"))?;
        }
        Ok(())
    }

    fn incomplete(&mut self, id: Option<u64>, start_us: u64, error: &str) -> Result<(), PackError> {
        writeln!(
            self.request_csv,
            "{},{start_us},{},false,,,,,,,,,,,,{}",
            id.map(|id| id.to_string()).unwrap_or_default(),
            elapsed(self.epoch)?,
            cell(error)
        )?;
        self.request_csv.flush()?;
        Ok(())
    }

    fn job(&mut self, prompt: &str, max_tokens: u32, measured: bool) -> Result<Request, PackError> {
        let started = Instant::now();
        let start_us = if measured && self.requests.is_empty() {
            self.epoch = started;
            0
        } else {
            elapsed(self.epoch)?
        };
        let sink = Arc::new(Sink {
            started,
            state: Mutex::new(SinkState {
                collector: Collector::default(),
                failure: None,
            }),
            wake: Condvar::new(),
        });
        let id = loop {
            match ai::generate(self.model.clone(), prompt.into(), max_tokens, sink.clone()) {
                Err(PackError::Mismatch(message))
                    if matches!(
                        message.as_str(),
                        "AI generation is busy"
                            | "AI callback dispatcher is busy"
                            | "AI delivery capacity exhausted"
                    ) && started.elapsed() < Duration::from_millis(250) =>
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Ok(id) => break id,
                Err(error) => {
                    if measured {
                        self.incomplete(None, start_us, &error.to_string())?;
                    }
                    return Err(error);
                }
            }
        };
        loop {
            let state = sink
                .state
                .lock()
                .map_err(|_| invalid("sink lock poisoned"))?;
            if let Some(error) = &state.failure {
                let error = error.clone();
                drop(state);
                ai::cancel(id)?;
                if measured {
                    self.incomplete(Some(id), start_us, &error.to_string())?;
                }
                return Err(error);
            }
            if let Some(request) = &state.collector.request {
                let mut request = request.clone();
                request.start_us = start_us;
                request.end_us = start_us
                    .checked_add(request.end_us)
                    .ok_or_else(|| invalid("request clock overflow"))?;
                return Ok(request);
            }
            if self.stop.load(Ordering::Acquire) || started.elapsed() >= Duration::from_secs(90) {
                drop(state);
                ai::cancel(id)?;
                if measured {
                    // Unknown terminal/token stats remain blank, never invented from text chunks.
                    self.incomplete(
                        Some(id),
                        start_us,
                        "interrupted or request deadline; terminal and token stats unknown",
                    )?;
                }
                return Err(invalid("interrupted or request exceeded ninety-second deadline; context-setup cancellation is not proven"));
            }
            let (state, _) = sink
                .wake
                .wait_timeout(state, Duration::from_millis(100))
                .map_err(|_| invalid("sink wait poisoned"))?;
            // Timestamp while the collector lock proves this job has not finished yet.
            // Otherwise a just-completed request could be absent from its own snapshot.
            let tick = if measured && state.collector.request.is_none() && state.failure.is_none() {
                Some(elapsed(self.epoch)?)
            } else {
                None
            };
            drop(state);
            if let Some(now) = tick {
                self.snapshots(now)?;
            }
        }
    }

    fn record(&mut self, request: Request) -> Result<(), PackError> {
        if self.requests.len() >= 4096 {
            return Err(invalid("request bound exceeded"));
        }
        let stats = &request.stats;
        writeln!(
            self.request_csv,
            "{},{},{},true,{},{},{},{},{},{},{},{},{},{},{:?},{}",
            request.id,
            request.start_us,
            request.end_us,
            request
                .wall_us()
                .ok_or_else(|| invalid("invalid request wall time"))?,
            stats.input_tokens,
            stats.output_tokens,
            request.chunks,
            request.bytes,
            stats.setup_us,
            stats.prefill_us,
            stats.decode_us,
            stats
                .first_token_us
                .map(|value| value.to_string())
                .unwrap_or_default(),
            request
                .first_receipt_us
                .map(|value| value.to_string())
                .unwrap_or_default(),
            request.reason,
            cell(request.error.as_deref().unwrap_or(""))
        )?;
        self.request_csv.flush()?;
        self.requests.push(request);
        self.snapshots(elapsed(self.epoch)?)
    }

    fn calibrate(&mut self, csv: &mut File) -> Result<String, PackError> {
        let mut repeats = 1000_usize;
        let mut seen = std::collections::HashSet::new();
        for probe in 1..=16 {
            if !seen.insert(repeats) {
                break;
            }
            let prompt = format!(
                "Explain this offline-library exercise clearly:\n{}\n/no_think",
                " x".repeat(repeats)
            );
            if prompt.len() > 16_384 {
                return Err(invalid("calibration prompt byte bound"));
            }
            let request = self.job(&prompt, 1, false)?;
            writeln!(
                csv,
                "{probe},{repeats},{},{:?},{}",
                request.stats.input_tokens,
                request.reason,
                cell(request.error.as_deref().unwrap_or(""))
            )?;
            csv.flush()?;
            if !matches!(request.reason, GenerationEnd::Eos | GenerationEnd::Limit)
                || request.error.is_some()
            {
                return Err(invalid("calibration native job failed"));
            }
            match calibration_next(repeats, request.stats.input_tokens)? {
                None => return Ok(prompt),
                Some(next) => repeats = next,
            }
        }
        Err(PackError::Mismatch("could not calibrate exactly two thousand rendered input tokens in sixteen bounded probes".into()))
    }
}

pub(super) fn run(config: Config) -> Result<(), PackError> {
    fs::create_dir(&config.output)?; // New directory only: preserve every earlier candidate/result.
    let mut summary = serde_json::json!({ "status":"running", "platform":"desktop", "os":std::env::consts::OS,
        "package_version":env!("CARGO_PKG_VERSION"), "pack_id":config.id, "manifest":config.manifest,
        "minimum_measurement_seconds":config.seconds, "smoke":config.smoke, "input_tokens":2000, "output_cap":128,
        "context_reset_each_request":true, "phone_gate_closed":false,
        "memory":"external Windows measured_process sampler required; this command does not report PSS or battery temperature",
        "timers":"request wall/first sink receipt use call start; Core first UTF8-ready uses accepted-job start; generation phase includes sampling/UTF8/enqueue waits",
        "snapshots":"latest completed request wall TPS, not throughput within the thirty-second interval or native-only decode TPS" });
    let save = |summary: &serde_json::Value| -> Result<(), PackError> {
        let bytes =
            serde_json::to_vec_pretty(summary).map_err(|error| invalid(&error.to_string()))?;
        write_atomic(&config.output.join("summary.json"), &bytes)
    };
    save(&summary)?;
    let result = execute(&config, &mut summary);
    if let Err(error) = &result {
        summary["status"] = "failed".into();
        summary["error"] = error.to_string().into();
    }
    save(&summary)?;
    result
}

fn execute(config: &Config, summary: &mut serde_json::Value) -> Result<(), PackError> {
    let manifest = Manifest::load(&config.manifest)?;
    let pack = manifest
        .packs
        .iter()
        .find(|pack| pack.id == config.id)
        .ok_or_else(|| PackError::NotFound(config.id.clone()))?;
    let verified = VerifiedModel::load(
        manifest_dir(&config.manifest),
        pack,
        ModelKind::Generation,
        |pin| {
            summary["model"] = pin.clone();
            Ok(())
        },
    )?;
    let model = Arc::clone(&verified.handle);
    let info = model.info()?;
    summary["model_info"] = serde_json::json!({ "training_context_tokens":info.training_context_tokens, "embedding_dimensions":info.embedding_dimensions, "estimated_reserved_bytes":info.estimated_reserved_bytes, "estimate_is_measured_memory":false });
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Release))
        .map_err(|error| invalid(&format!("signal handler: {error}")))?;
    let new_file = |name| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(config.output.join(name))
            .map_err(PackError::from)
    };
    let mut request_csv = new_file("requests.csv")?;
    writeln!(request_csv, "id,start_us,end_us,terminal_observed,wall_us,input_tokens,output_tokens,utf8_chunks,utf8_bytes,setup_us,prefill_phase_us,generation_phase_us,first_utf8_ready_us,first_sink_receipt_us,end,error")?;
    let mut snapshot_csv = new_file("snapshots.csv")?;
    writeln!(snapshot_csv, "scheduled_us,observed_us,latest_completed_id,request_start_us,request_end_us,request_output_tokens,request_wall_us,latest_completed_request_wall_tokens_s")?;
    let mut calibration = new_file("calibration.csv")?;
    writeln!(
        calibration,
        "probe,repetitions,rendered_input_tokens,end,error"
    )?;
    let mut runner = Runner {
        model: model.clone(),
        stop,
        epoch: Instant::now(),
        requests: Vec::new(),
        snapshots: Vec::new(),
        next_snapshot_us: 30_000_000,
        request_csv,
        snapshot_csv,
    };
    let result = (|| {
        let prompt = runner.calibrate(&mut calibration)?;
        new_file("prompt.txt")?.write_all(prompt.as_bytes())?;
        runner.epoch = Instant::now();
        let target = config
            .seconds
            .checked_mul(1_000_000)
            .ok_or_else(|| invalid("duration overflow"))?;
        let mut measured_wall = 0_u64;
        while measured_wall < target {
            let request = runner.job(&prompt, 128, true)?;
            measured_wall = measured_wall
                .checked_add(
                    request
                        .wall_us()
                        .ok_or_else(|| invalid("request wall duration"))?,
                )
                .ok_or_else(|| invalid("duration overflow"))?;
            let successful = request.stats.input_tokens == 2000
                && request.stats.output_tokens > 0
                && request.first_receipt_us.is_some()
                && matches!(request.reason, GenerationEnd::Eos | GenerationEnd::Limit)
                && request.error.is_none();
            runner.record(request)?;
            if !successful {
                return Err(invalid("measured request failed or lacked exact two-thousand-token input/nonempty output"));
            }
        }
        runner.snapshots(elapsed(runner.epoch)?)?;
        let duration = elapsed(runner.epoch)?;
        let assessment = assess(&runner.requests, &runner.snapshots, duration, config.smoke);
        let passed = assessment.passed;
        summary["assessment"] =
            serde_json::to_value(assessment).map_err(|error| invalid(&error.to_string()))?;
        summary["measured_elapsed_us"] = duration.into();
        summary["measured_request_wall_us"] = measured_wall.into();
        summary["completed_requests"] = runner.requests.len().into();
        summary["status"] = if config.smoke {
            "smoke_completed_gate_ineligible"
        } else {
            "completed"
        }
        .into();
        if !config.smoke && !passed {
            return Err(PackError::Mismatch(
                "desktop completed-request functional series failed; see summary reasons".into(),
            ));
        }
        Ok(())
    })();
    result.and(verified.close())
}
