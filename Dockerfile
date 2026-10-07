# One image, several entry points: `serve` (HTTP) and `worker` (learning loop) share the binary,
# so they can never disagree about schema or policy.
#   docker run -e PRB_DB=postgres://... -e PRB_ADMIN_TOKEN=... -p 8080:8080 pratibimba   # serve + /app
#   docker run -e PRB_DB=postgres://... pratibimba worker                   # learn
#   docker run -e PRB_DB=postgres://... -v $PWD/site.json:/site.json pratibimba apply /site.json
#   docker run -e PRB_DB=postgres://... -v $PWD/backups:/backups pratibimba backup /backups   # SQL dump

# The dashboard is a static export; Core serves it at /app.
FROM node:24-trixie-slim AS dashboard
WORKDIR /dash
COPY dashboard/package.json dashboard/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY dashboard ./
RUN npm run build


FROM rust:1-trixie AS build
WORKDIR /src

# Dependencies first, so a source-only change does not rebuild the tree.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml core/Cargo.toml
RUN mkdir -p core/src && echo 'fn main() {}' > core/src/main.rs && touch core/src/lib.rs \
 && cargo build --release -p pratibimba && rm -rf core/src

COPY core core
# The client script is embedded in the binary with include_str!.
COPY packages/client/sdk.js packages/client/sdk.js
# cargo decides on mtime; the stub files above are newer than the real ones would be.
RUN touch core/src/main.rs core/src/lib.rs && cargo build --release -p pratibimba


FROM debian:trixie-slim
# postgresql-client: pg_dump and psql for `pratibimba backup` / `restore`.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates postgresql-client \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home pratibimba
COPY --from=build /src/target/release/pratibimba /usr/local/bin/pratibimba
COPY --from=dashboard /dash/out /usr/share/pratibimba/dashboard
USER pratibimba
# The dashboard API stays off (503) until PRB_ADMIN_TOKEN is set.
ENV PRB_BIND=0.0.0.0:8080 PRB_DASHBOARD_DIR=/usr/share/pratibimba/dashboard
EXPOSE 8080
ENTRYPOINT ["pratibimba"]
CMD ["serve"]
