//! Private conversation persistence; never a LAN catalog or Notes fallback.
use crate::{content::storage, download::write_atomic, pack::PackError, reading::Destination};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_DOCUMENT: usize = 4 * 1024 * 1024;
const MAX_SESSIONS: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Message {
    role: String,
    content: String,
    context: bool,
    timestamp: u64,
    truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grounding: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Save {
    revision: u64,
    title: String,
    model: String,
    pack: Option<String>,
    messages: Vec<Message>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Record {
    id: String,
    revision: u64,
    title: String,
    model: String,
    pack: Option<String>,
    messages: Vec<Message>,
    updated: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    sessions: Vec<Record>,
}
struct Store {
    root: PathBuf,
}

fn id_valid(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn corrupt() -> PackError {
    PackError::Corrupt("Invalid private chat history".into())
}
fn conflict() -> PackError {
    PackError::Mismatch("Private chat history changed".into())
}
fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.contains('\0')
}
fn grounding_valid(value: &Value, pack: Option<&str>) -> bool {
    let Some(pack) = pack else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    if object
        .keys()
        .any(|key| !["pack", "sources", "citations", "flags", "no_sources"].contains(&key.as_str()))
        || value["pack"].as_str() != Some(pack)
        || object
            .get("no_sources")
            .is_some_and(|value| !value.is_boolean())
    {
        return false;
    }
    let (Some(sources), Some(citations), Some(flags)) = (
        value["sources"].as_array(),
        value["citations"].as_array(),
        value["flags"].as_object(),
    ) else {
        return false;
    };
    sources.len() <= 3
        && citations.len() <= 3
        && flags.len() == 4
        && ["uncited", "invalid_citations", "incomplete", "truncated"]
            .iter()
            .all(|key| value["flags"][key].is_boolean())
        && sources.iter().enumerate().all(|(index, source)| {
            source.as_object().is_some_and(|object| object.len() == 4)
                && source["number"].as_u64() == Some(index as u64 + 1)
                && source["title"].as_str().is_some_and(|title| {
                    !title.trim().is_empty()
                        && title.chars().count() <= 512
                        && !title.contains('\0')
                })
                && source["excerpt"].as_str().is_some_and(|excerpt| {
                    excerpt.chars().count() <= 1024 && !excerpt.contains('\0')
                })
                && source["url"].as_str().is_some_and(|url| {
                    (url.starts_with("C/") || url.starts_with("A/"))
                        && Destination::Article {
                            pack: pack.into(),
                            url: url.into(),
                        }
                        .valid()
                })
        })
        && citations.iter().enumerate().all(|(index, citation)| {
            citation
                .as_u64()
                .is_some_and(|number| number > 0 && number <= sources.len() as u64)
                && !citations[..index].contains(citation)
        })
}
impl Record {
    fn valid(&self) -> bool {
        if !id_valid(&self.id)
            || self.revision == 0
            || self.updated > 9_007_199_254_740_991
            || !text(&self.title, 512)
            || self.title.chars().any(char::is_control)
            || !super::identifier(&self.model)
            || self
                .pack
                .as_ref()
                .is_some_and(|pack| super::super::PackId::new(pack.clone()).is_err())
            || self.messages.is_empty()
            || self.messages.len() > 128
        {
            return false;
        }
        let mut context_turns = 0usize;
        let mut bytes = 0usize;
        for message in &self.messages {
            bytes = bytes.saturating_add(message.content.len());
            if !["user", "assistant"].contains(&message.role.as_str())
                || message.content.contains('\0')
                || bytes > 256 * 1024
                || message.timestamp > 9_007_199_254_740_991
                || message.grounding.as_ref().is_some_and(|value| {
                    message.role != "assistant" || !grounding_valid(value, self.pack.as_deref())
                })
            {
                return false;
            }
            if message.context {
                if message.role
                    != if context_turns.is_multiple_of(2) {
                        "user"
                    } else {
                        "assistant"
                    }
                    || message.content.trim().is_empty()
                {
                    return false;
                }
                context_turns += 1;
            }
        }
        // Preserve completed transcripts beyond the model's next-question limit.
        context_turns.is_multiple_of(2)
    }
}
impl Store {
    fn open(root: &Path) -> Result<Self, PackError> {
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    fn path(&self, name: &str) -> Result<PathBuf, PackError> {
        storage::path(&self.root, &format!("packs/private/{name}"))
    }
    fn read(&self) -> Result<Document, PackError> {
        let file = match File::open(self.path("ai-history.json")?) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Document {
                    version: 1,
                    sessions: vec![],
                })
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_DOCUMENT as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_DOCUMENT {
            return Err(corrupt());
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
        if document.version != 1
            || document.sessions.len() > MAX_SESSIONS
            || document.sessions.iter().any(|session| !session.valid())
            || document
                .sessions
                .iter()
                .enumerate()
                .any(|(index, session)| {
                    document.sessions[..index]
                        .iter()
                        .any(|other| other.id == session.id)
                })
        {
            return Err(corrupt());
        }
        Ok(document)
    }
    fn write(&self, document: &Document) -> Result<(), PackError> {
        let bytes = serde_json::to_vec(document).map_err(|_| corrupt())?;
        if bytes.len() > MAX_DOCUMENT {
            return Err(PackError::NoSpace);
        }
        let path = self.path("ai-history.json")?;
        write_atomic(&path, &bytes)?;
        storage::durable(&path)
    }
    fn list(&self) -> Result<Vec<Record>, PackError> {
        let _lock = storage::lock(&self.path("ai-history.lock")?)?;
        Ok(self.read()?.sessions)
    }
    fn get(&self, id: &str) -> Result<Record, PackError> {
        if !id_valid(id) {
            return Err(conflict());
        }
        self.list()?
            .into_iter()
            .find(|session| session.id == id)
            .ok_or_else(|| PackError::NotFound("Conversation unavailable".into()))
    }
    fn save(&self, id: &str, input: Save) -> Result<Record, PackError> {
        let revision = input.revision.checked_add(1).ok_or_else(conflict)?;
        let record = Record {
            id: id.into(),
            revision,
            title: input.title,
            model: input.model,
            pack: input.pack,
            messages: input.messages,
            updated: u64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| corrupt())?
                    .as_millis(),
            )
            .map_err(|_| corrupt())?,
        };
        if !record.valid() {
            return Err(conflict());
        }
        let _lock = storage::lock(&self.path("ai-history.lock")?)?;
        let mut document = self.read()?;
        let old = document
            .sessions
            .iter()
            .position(|session| session.id == id);
        if old.map_or(input.revision != 0, |index| {
            document.sessions[index].revision != input.revision
        }) {
            return Err(conflict());
        }
        if let Some(index) = old {
            document.sessions.remove(index);
        } else if document.sessions.len() >= MAX_SESSIONS {
            return Err(PackError::NoSpace);
        }
        document.sessions.insert(0, record.clone());
        self.write(&document)?;
        Ok(record)
    }
    fn remove(&self, id: &str, revision: u64, confirmed: bool) -> Result<(), PackError> {
        if !id_valid(id) || !confirmed {
            return Err(conflict());
        }
        let _lock = storage::lock(&self.path("ai-history.lock")?)?;
        let mut document = self.read()?;
        let index = document
            .sessions
            .iter()
            .position(|session| session.id == id && session.revision == revision)
            .ok_or_else(conflict)?;
        document.sessions.remove(index);
        self.write(&document)
    }
}

async fn work(
    state: std::sync::Arc<super::ServerState>,
    operation: impl FnOnce(Store) -> Result<Value, PackError> + Send + 'static,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let result =
        tokio::task::spawn_blocking(move || operation(Store::open(&state.data_dir)?)).await;
    match result {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        result => {
            let status = match result {
                Ok(Err(PackError::Mismatch(_))) => axum::http::StatusCode::CONFLICT,
                Ok(Err(PackError::NotFound(_))) => axum::http::StatusCode::NOT_FOUND,
                Ok(Err(PackError::NoSpace)) => axum::http::StatusCode::INSUFFICIENT_STORAGE,
                _ => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            };
            super::failure(status, "ai_history_unavailable")
        }
    }
}
pub(in crate::server) async fn listing(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::ServerState>>,
) -> axum::response::Response {
    work(state, |store| {
        let sessions = store.list()?.into_iter().map(|record| serde_json::json!({
            "id":record.id,"revision":record.revision,"title":record.title,"model":record.model,"pack":record.pack,"updated":record.updated
        })).collect::<Vec<_>>();
        Ok(serde_json::json!({"sessions":sessions}))
    }).await
}
pub(in crate::server) async fn get(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::ServerState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Response {
    work(state, move |store| {
        serde_json::to_value(store.get(&id)?).map_err(|_| corrupt())
    })
    .await
}
pub(in crate::server) async fn save(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::ServerState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::Json(input): axum::Json<Save>,
) -> axum::response::Response {
    work(state, move |store| {
        serde_json::to_value(store.save(&id, input)?).map_err(|_| corrupt())
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Remove {
    revision: u64,
    confirmed: bool,
}
pub(in crate::server) async fn remove(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::ServerState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::Json(input): axum::Json<Remove>,
) -> axum::response::Response {
    work(state, move |store| {
        store.remove(&id, input.revision, input.confirmed)?;
        Ok(serde_json::json!({"removed":true}))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn save(revision: u64) -> Save {
        serde_json::from_value(json!({"revision":revision,"title":"Test conversation","model":"local",
            "pack":null,"messages":[
                {"role":"user","content":"Remember cedar.","context":true,"timestamp":1,"truncated":false},
                {"role":"assistant","content":"Cedar.","context":true,"timestamp":2,"truncated":false}]})).unwrap()
    }
    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn history_retains_the_reply_to_the_last_admissible_native_question() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut record = save(0);
        record.messages = (0..34)
            .map(|index| Message {
                role: if index % 2 == 0 {
                    "user".into()
                } else {
                    "assistant".into()
                },
                content: format!("Turn {index}"),
                context: true,
                timestamp: index,
                truncated: false,
                grounding: None,
            })
            .collect();
        store
            .save(ID, record)
            .expect("33 admitted input messages plus the completed assistant reply must be saved");
        assert_eq!(store.get(ID).unwrap().messages.len(), 34);
    }

    #[test]
    fn saved_history_reopens_and_stale_writes_and_deletes_preserve_it() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("notes.db"), b"unchanged vault boundary").unwrap();
        let store = Store::open(root.path()).unwrap();
        assert!(store.list().unwrap().is_empty());
        let first = store.save(ID, save(0)).unwrap();
        assert_eq!(first.revision, 1);
        assert!(store.save(ID, save(0)).is_err());
        assert!(store.remove(ID, 0, true).is_err());
        assert!(store.remove(ID, 1, false).is_err());
        let reopened = Store::open(root.path()).unwrap();
        assert_eq!(
            reopened.get(ID).unwrap().messages[0].content,
            "Remember cedar."
        );
        assert_eq!(reopened.save(ID, save(1)).unwrap().revision, 2);
        reopened.remove(ID, 2, true).unwrap();
        assert!(reopened.list().unwrap().is_empty());
        assert_eq!(
            std::fs::read(root.path().join("notes.db")).unwrap(),
            b"unchanged vault boundary"
        );
    }

    #[test]
    fn malformed_history_or_source_links_cannot_replace_the_previous_document() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        store.save(ID, save(0)).unwrap();
        let path = root.path().join("packs/private/ai-history.json");
        let before = std::fs::read(&path).unwrap();
        let mut bad = save(1);
        bad.messages[1].role = "system".into();
        assert!(store.save(ID, bad).is_err());
        let mut bad = save(1);
        bad.pack = Some("book".into());
        bad.messages[1].grounding = Some(json!({"pack":"book","sources":[
            {"number":1,"title":"Bad","url":"https://example.invalid/","excerpt":"x"}],
            "citations":[1],"flags":{"uncited":false,"invalid_citations":false,"incomplete":false,"truncated":false}}));
        assert!(store.save(ID, bad).is_err());
        assert!(store.save("../notes.db", save(0)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"broken").unwrap();
        assert!(store.save(ID, save(1)).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"broken");
    }

    #[test]
    fn canonical_sources_survive_reopen_and_capacity_never_discards_old_sessions() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let mut first = save(0);
        first.pack = Some("book".into());
        let grounding = json!({"pack":"book","sources":[{"number":1,"title":"Article","url":"C/Article","excerpt":"Saved passage"}],
            "citations":[1],"flags":{"uncited":false,"invalid_citations":false,"incomplete":false,"truncated":false}});
        first.messages[1].grounding = Some(grounding.clone());
        store.save(ID, first).unwrap();
        assert_eq!(
            Store::open(root.path()).unwrap().get(ID).unwrap().messages[1].grounding,
            Some(grounding)
        );
        for index in 0..63 {
            store.save(&format!("{index:032x}"), save(0)).unwrap();
        }
        let path = root.path().join("packs/private/ai-history.json");
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            store.save(&format!("{:032x}", 1000), save(0)),
            Err(PackError::NoSpace)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut oversized = save(1);
        oversized.messages[0].content = "x".repeat(256 * 1024);
        assert!(store.save(ID, oversized).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(store.list().unwrap().len(), 64);
    }
}
