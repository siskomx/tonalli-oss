use super::*;
use crate::{
    index::{self, Index},
    pack::PreadSource,
};
use std::sync::mpsc;

type Gate = (mpsc::SyncSender<()>, mpsc::Receiver<()>);
struct GatedSource {
    source: PreadSource,
    gate: Mutex<Option<Gate>>,
}
impl PackSource for GatedSource {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, PackError> {
        let gate = self.gate.lock().unwrap().take();
        if let Some((entered, release)) = gate {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        self.source.read_at(offset, buffer)
    }
}

#[test]
fn validated_index_cannot_publish_after_registered_owner_is_removed_and_reinserted() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/articles/benign-v1.zim");
    let directory = tempfile::tempdir().unwrap();
    let independent = Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap();
    index::build(&independent, directory.path(), 1).unwrap();
    let index = Arc::new(Index::open(directory.path(), &independent).unwrap());
    let source = Arc::new(GatedSource {
        source: PreadSource::open(&path).unwrap(),
        gate: Mutex::new(None),
    });
    let registered = Arc::new(Zim::open(source.clone()).unwrap());
    let replacement = Arc::new(Zim::open(Arc::new(PreadSource::open(&path).unwrap())).unwrap());
    let registry = Arc::new(PackRegistry::default());
    registry
        .register_zim(PackId::new("reader").unwrap(), registered)
        .unwrap();
    let (entered, blocked) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    *source.gate.lock().unwrap() = Some((entered, gate));
    let attaching = Arc::clone(&registry);
    let worker =
        thread::spawn(move || attaching.attach_index(PackId::new("reader").unwrap(), index));
    blocked.recv_timeout(Duration::from_secs(5)).unwrap();
    let changing = Arc::clone(&registry);
    let (finished, observed) = mpsc::sync_channel(1);
    let replacement_worker = thread::spawn(move || {
        changing.remove(&PackId::new("reader").unwrap()).unwrap();
        changing
            .register_zim(PackId::new("reader").unwrap(), replacement)
            .unwrap();
        finished.send(()).unwrap();
    });
    let mutation_finished_without_io_lock = observed.recv_timeout(Duration::from_secs(3)).is_ok();
    release.send(()).unwrap();
    replacement_worker.join().unwrap();
    let result = worker.join().unwrap();
    assert!(
        mutation_finished_without_io_lock,
        "registry lock must not span index/source validation"
    );
    assert!(
        matches!(result, Err(PackError::Mismatch(_))),
        "same identity but different registered Arc must invalidate pending attachment"
    );
}
