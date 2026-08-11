FROM rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder
WORKDIR /workspace
COPY . .
RUN cargo build --locked --release --bin meta-agent-control-plane

FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS runtime
ARG VCS_REF=unknown
LABEL org.opencontainers.image.source="https://github.com/meta-agents-demo/meta-agent-control-plane.rs" \
      org.opencontainers.image.revision="${VCS_REF}"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home meta-agent
COPY --from=builder /workspace/target/release/meta-agent-control-plane /usr/local/bin/meta-agent-control-plane
COPY scripts/control_plane_secret_entrypoint.sh /usr/local/bin/meta-agent-secret-entrypoint
RUN chmod 0555 /usr/local/bin/meta-agent-control-plane /usr/local/bin/meta-agent-secret-entrypoint
USER 10001:10001
EXPOSE 8787/tcp 8788/tcp 8789/udp
ENTRYPOINT ["/usr/local/bin/meta-agent-control-plane"]
CMD ["--http-addr", "0.0.0.0:8787", "--tcp-addr", "0.0.0.0:8788", "--udp-addr", "0.0.0.0:8789"]
