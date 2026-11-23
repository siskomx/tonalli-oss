use std::sync::Arc;
use tantivy::merge_policy::{MergePolicy, NoMergePolicy};
use tantivy::{IndexWriter, TantivyDocument, TantivyError};

use crate::pack::PackError;

pub(super) struct BuildWriter {
    writer: Option<IndexWriter>,
    queue_budget: usize,
    queued_bytes: usize,
    submissions: usize,
    drains: usize,
    failure: Option<PackError>,
    payload: Option<String>,
    merge_policy: Arc<dyn MergePolicy>,
}

impl BuildWriter {
    pub(super) fn new(writer: IndexWriter, queue_budget: usize) -> Self {
        let merge_policy = writer.get_merge_policy();
        // Own every merge explicitly: upstream's worker-error join path may
        // otherwise return before background merges have exited.
        writer.set_merge_policy(Box::new(NoMergePolicy));
        Self {
            writer: Some(writer),
            queue_budget,
            queued_bytes: 0,
            submissions: 0,
            drains: 0,
            failure: None,
            payload: None,
            merge_policy,
        }
    }

    #[cfg(test)]
    pub(super) fn submit(
        &mut self,
        document: TantivyDocument,
        payload_bytes: usize,
    ) -> Result<(), PackError> {
        self.submit_checkpoint(document, payload_bytes, None)
            .map(|_| ())
    }

    pub(super) fn submit_checkpoint(
        &mut self,
        document: TantivyDocument,
        payload_bytes: usize,
        payload: Option<String>,
    ) -> Result<bool, PackError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Err(initial) = self
            .writer
            .as_mut()
            .expect("live build writer")
            .add_document(document)
        {
            return Err(self.submission_failure(initial));
        }
        self.payload = payload;
        self.submissions = self.submissions.saturating_add(1);
        self.queued_bytes = self.queued_bytes.saturating_add(payload_bytes);
        // Preserve the existing byte-bound queue drain, with one article of overshoot.
        if self.queued_bytes >= self.queue_budget {
            if let Err(error) = self.commit() {
                return Err(self.remember("queue drain commit", error, None));
            }
            self.queued_bytes = 0;
            self.drains += 1;
            return Ok(true);
        }
        Ok(false)
    }

    fn commit(&mut self) -> Result<(), TantivyError> {
        let mut prepared = self
            .writer
            .as_mut()
            .expect("live build writer")
            .prepare_commit()?;
        if let Some(payload) = &self.payload {
            prepared.set_payload(payload);
        }
        prepared.commit()?;
        loop {
            let writer = self.writer.as_mut().expect("live build writer");
            let candidates = self
                .merge_policy
                .compute_merge_candidates(&writer.index().searchable_segment_metas()?);
            if candidates.is_empty() {
                return Ok(());
            }
            for candidate in candidates {
                writer.merge(&candidate.0).wait()?;
            }
        }
    }

    pub(super) fn finish_checkpoint(mut self, payload: String) -> Result<usize, PackError> {
        self.payload = Some(payload);
        self.finish()
    }

    pub(super) fn abort(mut self) -> Result<(), PackError> {
        self.writer
            .take()
            .expect("live build writer")
            .wait_merging_threads()
            .map_err(super::index_error)
    }

    fn submission_failure(&mut self, initial: TantivyError) -> PackError {
        // add_document exposes only a killed-channel error. Join before Drop
        // discards the original worker result; never publish this prepared commit.
        let detail = if matches!(initial, TantivyError::ErrorInThread(_)) {
            self.writer
                .as_mut()
                .expect("live build writer")
                .prepare_commit()
                .err()
                .map(|error| error.to_string())
        } else {
            None
        };
        self.remember("submission", initial, detail)
    }

    fn context(&self, phase: &str) -> String {
        format!("index build {phase} (accepted_submissions={}, queued_payload_bytes={}, completed_drains={})",
            self.submissions, self.queued_bytes, self.drains)
    }

    fn remember(
        &mut self,
        phase: &str,
        initial: TantivyError,
        detail: Option<String>,
    ) -> PackError {
        let detail = detail
            .map(|error| format!("; underlying worker error: {error}"))
            .unwrap_or_default();
        let failure = super::index_error(format!("{}: {initial}{detail}", self.context(phase)));
        self.failure = Some(failure.clone());
        failure
    }

    pub(super) fn finish(mut self) -> Result<usize, PackError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Err(error) = self.commit() {
            return Err(self.remember("final commit", error, None));
        }
        let context = self.context("merge wait");
        self.writer
            .take()
            .expect("live build writer")
            .wait_merging_threads()
            .map_err(|error| super::index_error(format!("{context}: {error}")))?;
        Ok(self.drains)
    }
}

impl Drop for BuildWriter {
    fn drop(&mut self) {
        // Error/panic exits must also release merge workers before the native
        // owner reports exit. The original operation error remains authoritative.
        if let Some(writer) = self.writer.take() {
            let _ = writer.wait_merging_threads();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    use tantivy::{doc, schema::TEXT, Index};

    #[test]
    fn checkpoint_commits_merge_default_candidates_without_losing_cursor_or_docs() {
        let mut builder = tantivy::schema::Schema::builder();
        let text = builder.add_text_field("text", TEXT);
        let index = Index::create_in_ram(builder.build());
        let mut state = BuildWriter::new(index.writer_with_num_threads(1, 15_000_000).unwrap(), 1);
        for cursor in 1..=24 {
            assert!(state
                .submit_checkpoint(
                    doc!(text => format!("checkpoint document {cursor}")),
                    32,
                    Some(cursor.to_string())
                )
                .unwrap());
        }
        assert_eq!(state.finish_checkpoint("24".into()).unwrap(), 24);
        assert_eq!(index.reader().unwrap().searcher().num_docs(), 24);
        assert_eq!(index.load_metas().unwrap().payload.as_deref(), Some("24"));
        assert!(
            index.searchable_segment_ids().unwrap().len() < 24,
            "default merge candidates must actually be compacted"
        );
    }

    #[test]
    fn unexpected_preparation_success_never_publishes_pending_documents() {
        let mut builder = tantivy::schema::Schema::builder();
        let text = builder.add_text_field("text", TEXT);
        let index = Index::create_in_ram(builder.build());
        let mut state = BuildWriter::new(
            index.writer_with_num_threads(1, 15_000_000).unwrap(),
            usize::MAX,
        );
        state.submit(doc!(text => "pending document"), 16).unwrap();
        let failure = state.submission_failure(TantivyError::ErrorInThread(
            "synthetic channel failure".into(),
        ));
        assert!(failure.to_string().contains("synthetic channel failure"));
        assert_eq!(index.reader().unwrap().searcher().num_docs(), 0);
        assert_eq!(
            state.submit(doc!(text => "must not submit"), 15),
            Err(failure.clone())
        );
        assert_eq!(state.finish(), Err(failure));
        assert_eq!(index.reader().unwrap().searcher().num_docs(), 0);
    }

    #[test]
    fn queue_drain_failure_is_also_sticky_without_publishing() {
        let mut builder = tantivy::schema::Schema::builder();
        let text = builder.add_text_field("text", TEXT);
        let mut index = Index::create_in_ram(builder.build());
        index.settings_mut().manual_doc_id_mapping = true;
        let mut state = BuildWriter::new(index.writer_with_num_threads(1, 15_000_000).unwrap(), 1);
        let failure = state
            .submit(doc!(text => "one document forces drain"), 25)
            .unwrap_err();
        assert!(failure
            .to_string()
            .contains("manual_doc_id_mapping must be set to false"));
        assert!(failure.to_string().contains("queue drain commit"));
        assert_eq!(
            state.submit(doc!(text => "later producer"), 14),
            Err(failure.clone())
        );
        assert_eq!(state.finish(), Err(failure));
        assert_eq!(index.reader().unwrap().searcher().num_docs(), 0);
    }

    #[test]
    fn asynchronous_worker_detail_remains_sticky_for_queued_producers() {
        let mut builder = tantivy::schema::Schema::builder();
        let text = builder.add_text_field("text", TEXT);
        let mut index = Index::create_in_ram(builder.build());
        // Deterministic ordinary worker error on automatic segment flush, not a panic.
        index.settings_mut().manual_doc_id_mapping = true;
        let state = Arc::new(Mutex::new(BuildWriter::new(
            index.writer_with_num_threads(1, 15_000_000).unwrap(),
            usize::MAX,
        )));
        let mut locked = state.lock().unwrap();
        let gate = Arc::new(Barrier::new(5));
        let queued: Vec<_> = (0..4)
            .map(|_| {
                let state = Arc::clone(&state);
                let gate = Arc::clone(&gate);
                std::thread::spawn(move || {
                    gate.wait();
                    state
                        .lock()
                        .unwrap()
                        .submit(doc!(text => "queued producer"), 15)
                        .unwrap_err()
                })
            })
            .collect();
        gate.wait();
        let failure = (0..100_000)
            .find_map(|i| {
                let value = format!("hellohappytaxpayerlongtokenblabla{i}");
                let bytes = value.len();
                locked.submit(doc!(text => value), bytes).err()
            })
            .expect("bounded unique documents must trigger automatic worker flush");
        drop(locked);
        let queued_failures: Vec<_> = queued
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert!(
            failure
                .to_string()
                .contains("manual_doc_id_mapping must be set to false"),
            "underlying worker error lost: {failure}"
        );
        assert!(failure.to_string().contains("An index writer was killed"));
        assert!(
            queued_failures.iter().all(|error| error == &failure),
            "queued producer replaced the first diagnostic"
        );
        assert_eq!(
            index.reader().unwrap().searcher().num_docs(),
            0,
            "diagnostic collection must never publish a commit"
        );
    }
}
