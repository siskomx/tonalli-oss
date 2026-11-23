use crate::{
    download::{ensure_regular_or_absent, write_atomic},
    pack::PackError,
};
use reqwest::{
    blocking::Client,
    header::{CONTENT_RANGE, ETAG, IF_MATCH, RANGE},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub url: String,
    pub etag: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    source: Snapshot,
    offset: u64,
    length: u64,
    committed: u64,
    sha256: String,
}

fn valid_response(
    range: &str,
    etag: &str,
    offset: u64,
    length: u64,
    size: u64,
    expected: &str,
) -> Result<(), PackError> {
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= size && length > 0)
        .ok_or_else(|| PackError::Corrupt("Invalid map range".into()))?
        - 1;
    if range != format!("bytes {offset}-{end}/{size}")
        || etag != expected
        || !etag.starts_with('"')
        || !etag.ends_with('"')
        || etag.len() < 3
    {
        return Err(PackError::Mismatch(
            "Map source changed; check the download again".into(),
        ));
    }
    Ok(())
}

/// Each range is checkpointed independently. Interrupted uncommitted tails are discarded;
/// previously acknowledged bytes are hashed before reuse, including complete cached ranges.
pub(super) fn open_range(
    root: &Path,
    source: &Snapshot,
    client: &Client,
    offset: u64,
    length: u64,
    mut checkpoint: impl FnMut(u64) -> Result<(), PackError>,
) -> Result<File, PackError> {
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= source.size && length > 0)
        .ok_or_else(|| PackError::Corrupt("Invalid map range".into()))?
        - 1;
    fs::create_dir_all(root)?;
    let path = root.join(format!("{offset}-{length}.range"));
    let metadata = root.join(format!("{offset}-{length}.json"));
    ensure_regular_or_absent(&path)?;
    ensure_regular_or_absent(&metadata)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    let mut done = 0;
    let mut digest = Sha256::new();
    if metadata.try_exists()? {
        if fs::metadata(&metadata)?.len() > 8192 {
            return Err(PackError::Corrupt("Map checkpoint too large".into()));
        }
        let row: Checkpoint = serde_json::from_slice(&fs::read(&metadata)?)
            .map_err(|_| PackError::Corrupt("Invalid map checkpoint".into()))?;
        if row.source != *source
            || row.offset != offset
            || row.length != length
            || row.committed > length
            || row.committed > file.metadata()?.len()
        {
            return Err(PackError::Mismatch(
                "Map checkpoint does not match source".into(),
            ));
        }
        let mut remaining = row.committed;
        let mut buffer = [0; 65536];
        while remaining > 0 {
            let take = remaining.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..take])?;
            digest.update(&buffer[..take]);
            remaining -= take as u64;
            checkpoint(0)?;
        }
        if format!("{:x}", digest.clone().finalize()) != row.sha256 {
            return Err(PackError::Checksum);
        }
        done = row.committed;
    }
    file.set_len(done)?;
    file.seek(SeekFrom::Start(done))?;
    checkpoint(0)?;
    if done < length {
        let needed = length - done;
        if fs2::available_space(root)? < needed.saturating_add(16 * 1024 * 1024) {
            return Err(PackError::NoSpace);
        }
        let mut response = client
            .get(&source.url)
            .header("User-Agent", "Tonalli/0.1")
            .header(RANGE, format!("bytes={}-{end}", offset + done))
            .header(IF_MATCH, &source.etag)
            .send()
            .map_err(|_| PackError::Http("Map connection unavailable".into()))?;
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(PackError::Http(
                "Map source did not honor the requested range".into(),
            ));
        }
        valid_response(
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
                .unwrap_or(""),
            response
                .headers()
                .get(ETAG)
                .and_then(|h| h.to_str().ok())
                .unwrap_or(""),
            offset + done,
            needed,
            source.size,
            &source.etag,
        )?;
        let mut buffer = [0; 65536];
        let mut uncommitted = 0;
        while done < length {
            checkpoint(0)?;
            let take = (length - done).min(buffer.len() as u64) as usize;
            let count = response
                .read(&mut buffer[..take])
                .map_err(|_| PackError::Http("Map download interrupted".into()))?;
            if count == 0 {
                return Err(PackError::Truncated);
            }
            file.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            done += count as u64;
            uncommitted += count as u64;
            // Acknowledged progress always has a durable, verified restart point.
            if uncommitted >= 1024 * 1024 || done == length {
                file.sync_all()?;
                let row = Checkpoint {
                    source: source.clone(),
                    offset,
                    length,
                    committed: done,
                    sha256: format!("{:x}", digest.clone().finalize()),
                };
                write_atomic(
                    &metadata,
                    &serde_json::to_vec(&row)
                        .map_err(|_| PackError::Corrupt("Map checkpoint".into()))?,
                )?;
                checkpoint(uncommitted)?;
                uncommitted = 0;
            }
        }
        if response
            .read(&mut buffer[..1])
            .map_err(|_| PackError::Http("Map download interrupted".into()))?
            != 0
        {
            return Err(PackError::Corrupt("Map range too long".into()));
        }
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_changed_snapshot_and_incorrect_range() {
        assert!(
            valid_response("bytes 0-126/500", "\"changed\"", 0, 127, 500, "\"source\"").is_err()
        );
        assert!(
            valid_response("bytes 0-125/500", "\"source\"", 0, 127, 500, "\"source\"").is_err()
        );
        assert!(valid_response("bytes 0-126/500", "\"source\"", 0, 127, 500, "\"source\"").is_ok());
    }

    fn serve(bytes: Vec<u8>, offset: u64) -> (Snapshot, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/source", listener.local_addr().unwrap());
        let size = bytes.len() as u64;
        let task = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
            assert!(
                request.contains(&format!("range: bytes={offset}-{}", size - 1)),
                "{request}"
            );
            write!(socket, "HTTP/1.1 206 Partial Content\r\nETag: \"source\"\r\nContent-Range: bytes {offset}-{}/{size}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", size-1, size-offset).unwrap();
            let _ = socket.write_all(&bytes[offset as usize..]);
        });
        (
            Snapshot {
                url,
                size,
                etag: "\"source\"".into(),
            },
            task,
        )
    }

    #[test]
    fn paused_range_resumes_from_verified_prefix_and_rejects_corruption() {
        let root = tempfile::tempdir().unwrap();
        let bytes = vec![37; 2 * 1024 * 1024];
        let (mut source, task) = serve(bytes.clone(), 0);
        let client = Client::builder().no_proxy().build().unwrap();
        let result = open_range(root.path(), &source, &client, 0, source.size, |progress| {
            if progress > 0 {
                Err(PackError::Http("paused".into()))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        task.join().unwrap();
        let meta = root.path().join(format!("0-{}.json", source.size));
        let mut row: Checkpoint = serde_json::from_slice(&fs::read(&meta).unwrap()).unwrap();
        assert!(row.committed >= 1024 * 1024 && row.committed < source.size);
        let (resumed, task) = serve(bytes.clone(), row.committed);
        // Same immutable snapshot served by the new task-owned local listener.
        source.url = resumed.url;
        row.source = source.clone();
        fs::write(&meta, serde_json::to_vec(&row).unwrap()).unwrap();
        let path = root.path().join(format!("0-{}.range", source.size));
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"uncommitted tail")
            .unwrap();
        let mut file =
            open_range(root.path(), &source, &client, 0, source.size, |_| Ok(())).unwrap();
        let mut actual = Vec::new();
        file.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, bytes);
        drop(file);
        task.join().unwrap();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .write_all(b"tampered")
            .unwrap();
        assert!(matches!(
            open_range(root.path(), &source, &client, 0, source.size, |_| Ok(())),
            Err(PackError::Checksum)
        ));
    }
}
