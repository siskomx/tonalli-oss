# Optional Tonalli Chat broker

The Windows package includes its own checked Node runtime. Double-click
`Start Chat.cmd`; the browser opens when HTTPS is ready. Its generated owner
password appears once in that private console. The owner GUI selects local
network access (restart to apply), creates rooms/accounts and their grants, and
creates one-use browser invitation links/QRs or app setup codes. In Tonalli,
paste the app setup in Settings > Chat & Connections, check the address and
connect. Optional Remember keeps credentials in the native private encrypted
Chat store; restart never connects or forwards radio messages automatically.

Source integration is in progress; this is not yet a checked delivery.
One Node service uses Aedes MQTT3.1.1 over WSS and the shared Tonalli browser
client over HTTPS. Participants need an owner-created account and room grants;
radio hardware is optional. Radio/internet forwarding is off by default.

## Node package

Requires Node22 or later. Native reading/local/radio Chat never requires the user
to install Node. Build the helper with
`cargo build --locked --release -p tonalli-chat-store -j 1`; copy its executable
into `broker/bin/`. Build the existing web-stub UI and copy the complete dist
directory into `broker/public/`. Run `npm ci --ignore-scripts` in broker, retaining
licenses. Initialize an external private data directory once:

```
node src/init.mjs <private-directory>
node src/main.mjs <private-directory>
```

Initialization prints a generated owner password once and the certificate
SHA-256 fingerprint. Save the password securely; compare certificates before
continuing browser warnings. Never install a system-wide certificate. Defaults
bind only127.0.0.1:8443. LAN use requires an explicitly selected private bind
address in config.json and a certificate valid for its name/address. No router
forwarding. The GUI manages rooms/accounts/read/send/bridge grants separately
from ordinary participants. Stop with Ctrl+C. Restart never replays radio sends.

Keep the entire data directory private to the service account; on Windows use
a private user directory with its inherited user ACL. SQLCipher Chat history
uses history.db plus an external history.key. Back up both; loss of the key
loses history access. This service never opens Notes. Passwords use salted scrypt.
Never put data, secrets or keys in public/ or source control.

## Docker package

Build at repository root:
`docker build -f broker/Dockerfile -t tonalli-chat .`.
Initialize once with
`docker compose -f broker/compose.yaml run --rm chat node src/init.mjs /data --container`,
then `docker compose -f broker/compose.yaml up`. Its named private data volume
stays outside the source tree and is owned by container user1000. Back up this
volume, including its history key; do not delete it when updating the image.
Compose publishes loopback only; TONALLI_CONTAINER=1 permits
the internal container bind. Keep that host publication until explicitly choosing
a LAN/deployment configuration. Actual Compose checks cover TLS, authentication,
WSS messaging, unprivileged execution, normal Stop/restart, encrypted history and
wrong-key rejection without changing database bytes. Run
`node broker/scripts/docker-acceptance.mjs <image-tag>` from the repository root
to repeat these checks using an isolated private named volume. Test volumes are
retained for inspection. Distribution runtime versions and verbatim licenses are
in `licenses/linux-runtime`; no service keys or account data belong in an image.

Owned internet hosting uses mode`hosted` and a valid hostname/TLS certificate,
with explicit accounts/grants. No hosting/domain/purchase/deployment has occurred.
Native apps connect outbound over TLS; their private listeners stay private.

Advanced MQTT clients use WSS`/mqtt` and the same account credentials. Subscribe
only to authorized `tonalli/rooms/<id>/events`; publish bounded `{text,nonce}` to
`tonalli/rooms/<id>/send`. Nonces are unique16–80 character ASCII identifiers per
deliberate send. Retained/wildcard/cross-room writes are denied. Canonical sender
identity comes from authentication. Broker receipt/radio submission never proves
another person's receipt. Chat is not end-to-end encrypted across a gateway.

Android embedding remains unresolved: Node.js Mobile18.20.4 does not meet current
Aedes1.2.0's Node>=20 requirement. Android can connect to an owned broker while
its local room and radio remain independent. Exact native artifacts and combined
broker/browser/radio/Docker acceptance remain required before checked delivery.
