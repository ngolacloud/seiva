# A distroless, non-root runtime image for seiva-server.
#
# seiva-server is DORM-free (see its Cargo.toml), so the release binary builds
# from the public workspace with no private token. The binary is glibc-dynamic,
# so the runtime base is distroless `cc` (carries libgcc/libc), NOT `static`.
#
# Build from the repo root:
#   docker build -f deploy/seiva-server/Dockerfile -t ghcr.io/ngolacloud/seiva-server:0.1.0 .

# ---- build stage ------------------------------------------------------------
FROM rust:1.83-slim-bookworm AS build
WORKDIR /src
# System deps the workspace needs to compile (TLS, protobuf for seiva-grpc).
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
COPY . .
# Build ONLY the server crate (and its DORM-free dependency graph). ThinLTO/strip
# come from the workspace release profile.
RUN cargo build --release --locked -p seiva-server --bin seiva-server \
    && cp target/release/seiva-server /seiva-server

# ---- runtime stage ----------------------------------------------------------
# distroless `cc` nonroot: no shell, no package manager, runs as uid 65532.
FROM gcr.io/distroless/cc-debian12:nonroot
# Documents the default bind; the chart overrides SEIVA_HTTP_ADDR to 0.0.0.0.
ENV SEIVA_HTTP_ADDR=0.0.0.0:8080 \
    APP_ENV=production
COPY --from=build /seiva-server /usr/local/bin/seiva-server
EXPOSE 8080
# uid:gid 65532:65532 — the distroless `nonroot` user. Matches the chart's
# securityContext.runAsUser, so the filesystem and the pod agree.
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/seiva-server"]
