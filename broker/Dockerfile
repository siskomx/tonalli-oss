FROM rust:1.96.1-bookworm AS history
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN awk '/MemAvailable/ { if ($2 < 5242880) { print "Native build needs 5 GiB available"; exit 1 } }' /proc/meminfo \
    && cargo build --locked --release -p tonalli-chat-store -j 1

FROM node:22.23.2-bookworm-slim AS ui
WORKDIR /source/web-stub
COPY web-stub/package*.json ./
RUN npm ci --ignore-scripts --no-audit --no-fund
COPY web-stub ./
COPY branding.json /source/branding.json
COPY third_party/uppy-components-1.2.0-LICENSE /source/third_party/
COPY third_party/project-nomad/LICENSE third_party/project-nomad/UPSTREAM.json /source/third_party/project-nomad/
RUN npm run build

FROM node:22.23.2-bookworm-slim
ENV NODE_ENV=production
ENV TONALLI_CONTAINER=1
WORKDIR /app
COPY broker/package*.json ./
RUN npm ci --omit=dev --ignore-scripts --no-audit --no-fund
COPY broker/src ./src
COPY broker/licenses ./licenses
COPY android/licenses ./licenses/native
COPY LICENSE NOTICE LICENSES.md ./
COPY --from=history /source/target/release/tonalli-chat-store ./bin/tonalli-chat-store
COPY --from=ui /source/web-stub/dist ./public
RUN mkdir -p /data && chown node:node /data && chmod 700 /data
USER node
VOLUME /data
EXPOSE 8443
STOPSIGNAL SIGTERM
CMD ["node","src/main.mjs","/data"]
