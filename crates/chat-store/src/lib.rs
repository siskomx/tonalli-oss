//! Separate encrypted Chat journal shared by native gateways and the Node sidecar.
use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::Path;
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub struct Store {
    db: Connection,
}
impl Store {
    pub fn radio_metadata(&self) -> Result<Option<Value>, Error> {
        use rusqlite::OptionalExtension;
        let body: Option<String> = self
            .db
            .query_row("SELECT body FROM chat_radio WHERE id='last'", [], |r| {
                r.get(0)
            })
            .optional()?;
        body.map(|body| serde_json::from_str(&body).map_err(Into::into))
            .transpose()
    }
    pub fn save_radio_metadata(&mut self, value: &Value) -> Result<(), Error> {
        let body = serde_json::to_string(value)?;
        if !value.is_object() || body.len() > 32768 {
            return Err("Invalid radio metadata".into());
        }
        self.db.execute("INSERT INTO chat_radio(id,body) VALUES('last',?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",[body])?;
        Ok(())
    }
    pub fn connection(&self) -> Result<Option<Value>, Error> {
        use rusqlite::OptionalExtension;
        let body: Option<String> = self
            .db
            .query_row(
                "SELECT body FROM chat_connections WHERE id='broker'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        body.map(|body| {
            let value: Value = serde_json::from_str(&body)?;
            if !value.is_object() {
                return Err("Invalid stored connection".into());
            }
            Ok(value)
        })
        .transpose()
    }
    pub fn save_connection(&mut self, value: &Value) -> Result<(), Error> {
        if !value.is_object() {
            return Err("Invalid connection".into());
        }
        let body = serde_json::to_string(value)?;
        if body.len() > 32768 {
            return Err("Connection too large".into());
        }
        self.db.execute("INSERT INTO chat_connections(id,body) VALUES('broker',?1) ON CONFLICT(id) DO UPDATE SET body=excluded.body",[body])?;
        Ok(())
    }
    pub fn forget_connection(&mut self) -> Result<(), Error> {
        self.db
            .execute("DELETE FROM chat_connections WHERE id='broker'", [])?;
        Ok(())
    }
    pub fn open(path: &Path, key: &str) -> Result<Self, Error> {
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid key".into());
        }
        let keyed = |flags| -> Result<Connection, rusqlite::Error> {
            let c = Connection::open_with_flags(path, flags)?;
            c.pragma_update(None, "key", format!("x'{key}'"))?;
            c.pragma_update(None, "cipher_memory_security", "ON")?;
            Ok(c)
        };
        if path.exists() {
            let probe = keyed(OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            probe.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })?;
            let version: i64 = probe.pragma_query_value(None, "user_version", |r| r.get(0))?;
            if version != 1 {
                return Err("unsupported chat history".into());
            }
        }
        let db = keyed(OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE)?;
        let cipher: String = db.pragma_query_value(None, "cipher_version", |r| r.get(0))?;
        if cipher.is_empty() {
            return Err("SQLCipher required".into());
        }
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS chat_messages(id INTEGER PRIMARY KEY AUTOINCREMENT,room TEXT NOT NULL,actor TEXT NOT NULL,nonce TEXT NOT NULL,body TEXT NOT NULL,UNIQUE(room,actor,nonce));
        CREATE INDEX IF NOT EXISTS chat_room_id ON chat_messages(room,id);
        CREATE TABLE IF NOT EXISTS chat_connections(id TEXT PRIMARY KEY,body TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS chat_radio(id TEXT PRIMARY KEY,body TEXT NOT NULL); PRAGMA user_version=1;")?;
        Ok(Self { db })
    }
    pub fn append(
        &mut self,
        room: &str,
        actor: &str,
        nonce: &str,
        message: &Value,
    ) -> Result<(Value, bool), Error> {
        if !message.is_object() {
            return Err("invalid message".into());
        }
        if room.is_empty()
            || room.len() > 64
            || actor.is_empty()
            || actor.len() > 128
            || nonce.is_empty()
            || nonce.len() > 128
        {
            return Err("invalid message identity".into());
        }
        let body = serde_json::to_string(message)?;
        if body.len() > 8192 {
            return Err("message too large".into());
        }
        let tx = self.db.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO chat_messages(room,actor,nonce,body) VALUES(?1,?2,?3,?4)",
            params![room, actor, nonce, body],
        )? == 1;
        let (id, body): (i64, String) = tx.query_row(
            "SELECT id,body FROM chat_messages WHERE room=?1 AND actor=?2 AND nonce=?3",
            params![room, actor, nonce],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if id > 9_007_199_254_740_991 {
            return Err("chat sequence exhausted".into());
        }
        let mut value: Value = serde_json::from_str(&body)?;
        if !value.is_object() {
            return Err("invalid stored message".into());
        }
        if !inserted
            && ["text", "origin", "node", "channel"]
                .iter()
                .any(|field| value[*field] != message[*field])
        {
            return Err("message identity conflict".into());
        }
        tx.execute("DELETE FROM chat_messages WHERE room=?1 AND id<COALESCE((SELECT id FROM chat_messages WHERE room=?1 ORDER BY id DESC LIMIT 1 OFFSET 9999),0)",[room])?;
        tx.commit()?;
        value["id"] = json!(id);
        Ok((value, inserted))
    }
    pub fn list(&self, room: &str) -> Result<Vec<Value>, Error> {
        if room.is_empty() || room.len() > 64 {
            return Err("invalid room".into());
        }
        let mut statement=self.db.prepare("SELECT id,body FROM (SELECT id,body FROM chat_messages WHERE room=?1 ORDER BY id DESC LIMIT 100) ORDER BY id")?;
        let rows = statement.query_map([room], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut values = Vec::new();
        for row in rows {
            let (id, body) = row?;
            let mut value: Value = serde_json::from_str(&body)?;
            if !value.is_object() {
                return Err("invalid stored message".into());
            }
            value["id"] = json!(id);
            values.push(value);
        }
        Ok(values)
    }
    pub fn status(&mut self, id: i64, status: &str) -> Result<(), Error> {
        if !["radio_submitted", "delivery_unknown"].contains(&status) {
            return Err("invalid status".into());
        }
        let tx = self.db.transaction()?;
        let body: String =
            tx.query_row("SELECT body FROM chat_messages WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
        let mut value: Value = serde_json::from_str(&body)?;
        if !value.is_object() {
            return Err("invalid stored message".into());
        }
        value["status"] = json!(status);
        tx.execute(
            "UPDATE chat_messages SET body=?1 WHERE id=?2",
            params![serde_json::to_string(&value)?, id],
        )?;
        tx.commit()?;
        Ok(())
    }
}
