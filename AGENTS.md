# Contributor instructions

Preserve working features and user data. Keep the Rust core and shared React UI;
native platform adapters should stay thin. Reuse compatible existing code before
adding equivalent functionality, and preserve exact license/provenance notices.

Keep ordinary flows simple and English/Spanish labels understandable. Technical
controls belong in Settings/Advanced. Never commit local logs, private user paths,
device identifiers, credentials, databases, signing material or downloaded packs.

Serving is opt-in and LAN-only in native apps. Optional broker deployment is
separate. Never expose Notes/settings or arbitrary disk files. Use SQLCipher for
encrypted Notes and separate protected chat history; do not fall back to plaintext.
Removable sources use positioned reads. Stage, verify and atomically activate pack
and index changes. Return typed errors for corrupt inputs and removed media.

Make small focused changes, build as needed, and verify complete changed flows
plus necessary trust/data-integrity boundaries. Do not introduce speculative
infrastructure or repetitive broad test cycles. Document actual limitations.
