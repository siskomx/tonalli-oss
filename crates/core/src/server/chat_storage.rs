//! Native Chat's private SQLCipher store and external key, separate from Notes.
pub(super) fn open(
    directory: &std::path::Path,
) -> Result<tonalli_chat_store::Store, tonalli_chat_store::Error> {
    use std::io::Write;
    static INITIALIZE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = INITIALIZE.lock().map_err(|_| "Chat storage unavailable")?;
    let directory = directory.join("chat");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    let database = directory.join("chat.db");
    let keyfile = directory.join("chat.key");
    if !keyfile.exists() {
        if database.exists() {
            return Err("Chat history key is missing".into());
        }
        let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut bytes[..]).map_err(|_| "Chat randomness unavailable")?;
        let key =
            zeroize::Zeroizing::new(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&keyfile)?;
        file.write_all(key.as_bytes())?;
        file.sync_all()?;
    }
    let key = zeroize::Zeroizing::new(std::fs::read_to_string(keyfile)?);
    tonalli_chat_store::Store::open(&database, &key)
}
