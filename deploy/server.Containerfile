# The Rust server (kokoro-server) on distroless: glibc and libgcc, no shell. Build from the repo root:
#   docker build -t optimized-kokoro-server -f deploy/server.Containerfile .
# Data and config are not baked in: compose mounts UniDic and the config (with the API keys).
#
# Pinned: base images by digest, crates by Cargo.lock. The builder is Debian 12 like the runtime, so
# the binary needs no newer glibc than it has. .cargo/config.toml builds for x86-64-v3.
# Dependabot updates the FROM lines. Keep the Rust version and RUST_TOOLCHAIN in
# .github/workflows/ci.yml the same.

FROM docker.io/library/rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p kokoro-server && \
    install -D -m 0755 target/release/kokoro-server /out/kokoro-server && \
    mkdir -p /out/run/kokoro /out/run/secrets /out/unidic

FROM gcr.io/distroless/cc-debian12@sha256:e5d81ddde149641e2a9ba55be4545bc125c67de07508b03ba4c22e6eb0ded5aa
COPY --from=builder /out/kokoro-server /usr/local/bin/kokoro-server
# kokoro-espeak-ng reads its data at run time from the path it was built at (CARGO_MANIFEST_DIR).
COPY --from=builder /src/crates/espeak-ng/espeak-ng-data /src/crates/espeak-ng/espeak-ng-data
# Mount points (compose mounts the socket volume, the config secret and UniDic there).
COPY --from=builder /out/run /run
COPY --from=builder /out/unidic /unidic
# compose runs it as 65534:65534 too.
USER 65534:65534
ENTRYPOINT ["/usr/local/bin/kokoro-server"]
