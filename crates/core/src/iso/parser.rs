use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use libmkiso::source::{ReadAt, SourceCursor};

use crate::pack::{PackError, PackSource};

use super::{normalized_path, Extent, ExtentSource};

const MAX_IMAGE: u64 = 64 << 30;
const MAX_METADATA: u64 = 16 << 20;
const MAX_READ_BYTES: u64 = 64 << 20;
const MAX_ENTRIES: u64 = 100_000;
const MAX_DEPTH: usize = 64;
const CHUNK: usize = 64 * 1024;
const MAX_TOTAL: u64 = 128 << 30;
const MAX_EXTENTS: usize = MAX_METADATA as usize / std::mem::size_of::<Extent>();

#[derive(Default)]
struct IndexBudget {
    logical_bytes: u64,
    extents: usize,
}

#[derive(Clone)]
struct Positioned {
    source: Arc<dyn PackSource>,
    used: Arc<AtomicU64>,
}

impl ReadAt for Positioned {
    fn len(&self) -> u64 {
        self.source.len()
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len() || output.is_empty() {
            return Ok(0);
        }
        let count = output.len().min(CHUNK).min((self.len() - offset) as usize);
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(count as u64)
                    .filter(|next| *next <= MAX_READ_BYTES)
            })
            .map_err(|_| {
                io::Error::other(PackError::Mismatch(
                    "ISO discovery read budget exceeded".into(),
                ))
            })?;
        super::source::read_exact_at(&*self.source, offset, &mut output[..count])
            .map_err(io::Error::other)?;
        Ok(count)
    }
}

fn recognize(input: &Positioned) -> Result<(bool, bool), PackError> {
    let mut iso = false;
    let mut extended_area = false;
    let mut udf_namespace = false;
    // Recognition descriptors are contiguous; never search ordinary file payloads.
    for block in 16..(input.len() / 2048).min(256) {
        let mut signature = [0; 7];
        input
            .read_exact_at(block * 2048, &mut signature)
            .map_err(source_error)?;
        let identifier = &signature[1..6];
        if identifier == b"CD001" && signature[6] == 1 && matches!(signature[0], 0..=3 | 255) {
            iso = true;
            continue;
        }
        if signature[0] != 0 || signature[6] != 1 {
            break;
        }
        match identifier {
            b"BEA01" if !extended_area => {
                extended_area = true;
                udf_namespace = false;
            }
            b"NSR02" | b"NSR03" if extended_area => udf_namespace = true,
            b"TEA01" if extended_area => {
                if udf_namespace {
                    return Ok((iso, true));
                }
                extended_area = false;
            }
            b"BOOT2" | b"CDW02" => {}
            _ => break,
        }
    }
    if extended_area && udf_namespace {
        return Err(PackError::Corrupt(
            "incomplete UDF recognition sequence".into(),
        ));
    }
    Ok((iso, false))
}

pub(super) fn parse(
    source: Arc<dyn PackSource>,
) -> Result<BTreeMap<String, Vec<Extent>>, PackError> {
    if source.len() > MAX_IMAGE {
        return Err(PackError::Mismatch(
            "ISO image exceeds 64 GiB spike limit".into(),
        ));
    }
    if source.len() < 17 * 2048 {
        return Err(PackError::Truncated);
    }
    let input = Positioned {
        source: Arc::clone(&source),
        used: Arc::new(AtomicU64::new(0)),
    };
    // Bridge images can have an empty ISO tree and real files only in UDF.
    let (iso, udf) = recognize(&input)?;
    let mut files = BTreeMap::new();
    let mut budget = IndexBudget::default();
    if udf {
        let reader = libmkiso::udf::UdfReader::open_source_with_limits(
            input,
            libmkiso::udf::Limits {
                max_entries: MAX_ENTRIES,
                max_metadata_bytes: MAX_METADATA,
                max_entry_bytes: MAX_IMAGE,
                max_total_bytes: MAX_TOTAL,
                max_input_bytes: MAX_IMAGE,
                max_nesting_depth: MAX_DEPTH,
            },
            libmkiso::udf::SourceLimits {
                max_read_bytes: MAX_READ_BYTES,
                max_scratch_bytes: CHUNK,
                ..Default::default()
            },
        )
        .map_err(udf_error)?;
        for (index, entry) in reader.entries().iter().enumerate() {
            if entry.kind != libmkiso::udf::EntryKind::File {
                continue;
            }
            let mut extents = Vec::new();
            let remaining = MAX_EXTENTS - budget.extents;
            reader
                .visit_classified_extents(index, |_, kind, len| {
                    if extents.len() >= remaining {
                        return Err(libmkiso::udf::Error::ResourceLimit(
                            "owned extent budget exceeded",
                        ));
                    }
                    extents.push(match kind {
                        libmkiso::udf::UdfExtentKind::Recorded { source_offset } => {
                            Extent::Recorded {
                                offset: source_offset,
                                len,
                            }
                        }
                        libmkiso::udf::UdfExtentKind::Unallocated
                        | libmkiso::udf::UdfExtentKind::AllocatedUnrecorded => Extent::Zero { len },
                    });
                    Ok(())
                })
                .map_err(udf_error)?;
            insert(
                &mut files,
                &mut budget,
                &source,
                &entry.name,
                entry.size,
                extents,
            )?;
        }
    } else if iso {
        let reader = libmkiso::iso9660::IsoReader::open_with_options(
            SourceCursor::new(input),
            libmkiso::iso9660::ReadOptions {
                namespace: libmkiso::iso9660::Namespace::PreferRockRidge,
                limits: libmkiso::iso9660::Limits {
                    max_entries: MAX_ENTRIES,
                    max_metadata_bytes: MAX_METADATA,
                    max_nesting_depth: MAX_DEPTH,
                },
            },
        )
        .map_err(iso_error)?;
        for (entry, extents) in reader.entries().iter().zip(&reader.index().extents) {
            if entry.directory || entry.link_target.is_some() {
                continue;
            }
            if extents.len() > MAX_EXTENTS - budget.extents {
                return Err(PackError::Mismatch("owned extent budget exceeded".into()));
            }
            let extents = extents
                .iter()
                .map(|extent| Extent::Recorded {
                    offset: extent.offset,
                    len: extent.size,
                })
                .collect();
            insert(
                &mut files,
                &mut budget,
                &source,
                &entry.name,
                entry.size,
                extents,
            )?;
        }
    } else {
        return Err(PackError::Corrupt("no optical ISO/UDF signature".into()));
    }
    Ok(files)
}

fn insert(
    files: &mut BTreeMap<String, Vec<Extent>>,
    budget: &mut IndexBudget,
    source: &Arc<dyn PackSource>,
    name: &str,
    expected: u64,
    extents: Vec<Extent>,
) -> Result<(), PackError> {
    if expected > MAX_IMAGE {
        return Err(PackError::Mismatch(
            "optical file exceeds 64 GiB spike limit".into(),
        ));
    }
    let logical_bytes = budget
        .logical_bytes
        .checked_add(expected)
        .filter(|total| *total <= MAX_TOTAL)
        .ok_or_else(|| PackError::Mismatch("optical logical size budget exceeded".into()))?;
    let extent_count = budget
        .extents
        .checked_add(extents.len())
        .filter(|count| *count <= MAX_EXTENTS)
        .ok_or_else(|| PackError::Mismatch("owned extent budget exceeded".into()))?;
    let path = normalized_path(name)?;
    let validated = ExtentSource::new(Arc::clone(source), extents.clone())?;
    if validated.len() != expected || path.is_empty() || files.contains_key(path) {
        return Err(PackError::Corrupt(
            "invalid/duplicate optical file metadata".into(),
        ));
    }
    budget.logical_bytes = logical_bytes;
    budget.extents = extent_count;
    files.insert(path.to_owned(), extents);
    Ok(())
}

fn source_error(error: io::Error) -> PackError {
    if let Some(original) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<PackError>())
    {
        return original.clone();
    }
    if error.kind() == io::ErrorKind::UnexpectedEof {
        PackError::Truncated
    } else {
        PackError::Io(error.to_string())
    }
}

fn iso_error(error: libmkiso::iso9660::Error) -> PackError {
    use libmkiso::iso9660::Error;
    match error {
        Error::Io(error) => source_error(error),
        Error::Malformed(message) => PackError::Corrupt(message),
        Error::Unsupported(message) => PackError::Mismatch(message),
        Error::ResourceLimit(message) => PackError::Mismatch(message.into()),
    }
}

fn udf_error(error: libmkiso::udf::Error) -> PackError {
    use libmkiso::udf::Error;
    match error {
        Error::Io(error) => source_error(error),
        Error::Malformed(message) | Error::Integrity(message) => PackError::Corrupt(message),
        Error::Unsupported(message) => PackError::Mismatch(message),
        Error::ResourceLimit(message) => PackError::Mismatch(message.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OneByteSource(Vec<u8>);

    impl PackSource for OneByteSource {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
            if output.is_empty() {
                return Ok(0);
            }
            let byte = self.0.get(offset as usize).ok_or(PackError::Truncated)?;
            output[0] = *byte;
            Ok(1)
        }
    }

    fn empty_source() -> Arc<dyn PackSource> {
        Arc::new(OneByteSource(Vec::new()))
    }

    #[test]
    fn legal_short_reads_do_not_exhaust_metadata_budget() {
        let input = Positioned {
            source: Arc::new(OneByteSource(vec![0x5a; CHUNK])),
            used: Arc::new(AtomicU64::new(0)),
        };
        let mut bytes = vec![0; CHUNK];
        input.read_exact_at(0, &mut bytes).unwrap();
        assert!(bytes.iter().all(|byte| *byte == 0x5a));
    }

    #[test]
    fn file_size_limit_applies_to_every_namespace() {
        let mut files = BTreeMap::new();
        let mut budget = IndexBudget::default();
        assert!(matches!(
            insert(
                &mut files,
                &mut budget,
                &empty_source(),
                "large",
                MAX_IMAGE + 1,
                vec![Extent::Zero { len: MAX_IMAGE + 1 }]
            ),
            Err(PackError::Mismatch(_))
        ));
        assert!(files.is_empty());
    }

    #[test]
    fn aggregate_logical_limit_applies_to_every_namespace() {
        let mut files = BTreeMap::new();
        let mut budget = IndexBudget::default();
        for name in ["one", "two"] {
            insert(
                &mut files,
                &mut budget,
                &empty_source(),
                name,
                MAX_IMAGE,
                vec![Extent::Zero { len: MAX_IMAGE }],
            )
            .unwrap();
        }
        assert!(matches!(
            insert(
                &mut files,
                &mut budget,
                &empty_source(),
                "three",
                MAX_IMAGE,
                vec![Extent::Zero { len: MAX_IMAGE }]
            ),
            Err(PackError::Mismatch(_))
        ));
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn owned_extent_budget_is_shared_between_files() {
        let mut files = BTreeMap::new();
        let mut budget = IndexBudget::default();
        let count = MAX_METADATA as usize / std::mem::size_of::<Extent>();
        insert(
            &mut files,
            &mut budget,
            &empty_source(),
            "one",
            count as u64,
            vec![Extent::Zero { len: 1 }; count],
        )
        .unwrap();
        assert!(matches!(
            insert(
                &mut files,
                &mut budget,
                &empty_source(),
                "two",
                1,
                vec![Extent::Zero { len: 1 }]
            ),
            Err(PackError::Mismatch(_))
        ));
        assert_eq!(files.len(), 1);
    }
}
