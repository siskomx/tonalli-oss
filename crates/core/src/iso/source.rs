use std::sync::Arc;

use crate::pack::{PackError, PackSource};

use super::Extent;

struct Section {
    start: u64,
    extent: Extent,
}

/// Concatenated recorded/zero sections; reads allocate no payload buffers.
pub struct ExtentSource {
    source: Arc<dyn PackSource>,
    sections: Vec<Section>,
    len: u64,
}

impl ExtentSource {
    pub fn new(source: Arc<dyn PackSource>, extents: Vec<Extent>) -> Result<Self, PackError> {
        let mut len = 0_u64;
        let mut sections = Vec::with_capacity(extents.len());
        for extent in extents {
            if let Extent::Recorded { offset, len } = extent {
                if offset.checked_add(len).is_none_or(|end| end > source.len()) {
                    return Err(PackError::Mismatch("extent exceeds image source".into()));
                }
            }
            let start = len;
            len = len
                .checked_add(extent.len())
                .ok_or_else(|| PackError::Mismatch("extent length overflow".into()))?;
            if !extent.is_empty() {
                sections.push(Section { start, extent });
            }
        }
        Ok(Self {
            source,
            sections,
            len,
        })
    }
}

impl PackSource for ExtentSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize, PackError> {
        if offset
            .checked_add(output.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(PackError::Truncated);
        }
        if output.is_empty() {
            return Ok(0);
        }
        // Even an all-zero UDF range belongs to media that may have disappeared.
        probe(&*self.source)?;
        let first = self
            .sections
            .partition_point(|section| section.start + section.extent.len() <= offset);
        let mut filled = 0;
        let mut position = offset;
        for section in &self.sections[first..] {
            if filled == output.len() {
                break;
            }
            let local = position - section.start;
            let count = (section.extent.len() - local).min((output.len() - filled) as u64) as usize;
            let chunk = &mut output[filled..filled + count];
            match section.extent {
                Extent::Recorded { offset, .. } => {
                    read_exact_at(&*self.source, offset + local, chunk)?;
                }
                Extent::Zero { .. } => chunk.fill(0),
            }
            position += count as u64;
            filled += count;
        }
        if filled != output.len() {
            return Err(PackError::Truncated);
        }
        Ok(filled)
    }
}

pub(super) fn probe(source: &dyn PackSource) -> Result<(), PackError> {
    if source.is_empty() {
        return Ok(());
    }
    read_exact_at(source, 0, &mut [0])
}

pub(super) fn read_exact_at(
    source: &dyn PackSource,
    mut offset: u64,
    mut output: &mut [u8],
) -> Result<(), PackError> {
    while !output.is_empty() {
        let count = source.read_at(offset, output)?;
        if count == 0 || count > output.len() {
            return Err(PackError::Truncated);
        }
        offset = offset
            .checked_add(count as u64)
            .ok_or(PackError::Truncated)?;
        output = &mut output[count..];
    }
    Ok(())
}
