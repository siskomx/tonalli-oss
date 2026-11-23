//! Bounded private JSON-lines adapter; never accepts SQL or logs message/key values.
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use tonalli_chat_store::{Error, Store};
use zeroize::Zeroizing;
fn line(input: &mut impl BufRead) -> Result<Value, Error> {
    let mut bytes = Zeroizing::new(Vec::new());
    let count = std::io::Read::take(&mut *input, 32769).read_until(b'\n', &mut bytes)?;
    if count == 0 || count > 32768 || bytes.last() != Some(&b'\n') {
        return Err("invalid IPC frame".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn field<'a>(v: &'a Value, name: &str, max: usize) -> Result<&'a str, Error> {
    v[name]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max)
        .ok_or_else(|| "invalid field".into())
}
fn run() -> Result<(), Error> {
    let mut input = io::stdin().lock();
    let mut init = line(&mut input)?;
    let path = std::path::PathBuf::from(field(&init, "path", 4096)?);
    let Value::String(key) = init["key"].take() else {
        return Err("invalid key".into());
    };
    let key = Zeroizing::new(key);
    drop(init);
    let mut store = Store::open(&path, &key)?;
    drop(key);
    let mut out = io::stdout().lock();
    writeln!(out, "{}", json!({"ready":true}))?;
    out.flush()?;
    loop {
        let request = match line(&mut input) {
            Ok(v) => v,
            Err(_) => break,
        };
        let result = (|| -> Result<Value, Error> {
            let room = field(&request, "room", 64)?;
            match request["op"].as_str() {
                Some("append") => Ok(store
                    .append(
                        room,
                        field(&request, "actor", 128)?,
                        field(&request, "nonce", 128)?,
                        &request["message"],
                    )?
                    .0),
                Some("list") => Ok(json!(store.list(room)?)),
                _ => Err("unknown operation".into()),
            }
        })();
        let response = match result {
            Ok(value) => json!({"id":request["id"],"result":value}),
            Err(_) => json!({"id":request["id"],"error":"history operation failed"}),
        };
        writeln!(out, "{response}")?;
        out.flush()?;
    }
    Ok(())
}
fn main() {
    if run().is_err() {
        eprintln!("Encrypted chat history unavailable");
        std::process::exit(1);
    }
}
