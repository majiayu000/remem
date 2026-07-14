# Container image for running the remem MCP server over stdio.
#
# Purpose: let the Glama MCP directory (and anyone who prefers a container)
# build and introspect `remem mcp` without a local Rust toolchain. The primary
# install path for real usage remains the native binary + hooks (see README);
# this image only serves the MCP stdio transport.
#
# Build:  docker build -t remem-mcp .
# Run:    docker run --rm -i remem-mcp        # speaks MCP over stdio

# --- build stage -------------------------------------------------------------
FROM rust:1-bookworm AS builder

# rusqlite is compiled with `bundled-sqlcipher-vendored-openssl`, which builds
# SQLCipher and OpenSSL from source. That vendored OpenSSL build needs perl and
# make in addition to the C toolchain already present in the rust image.
RUN apt-get update \
    && apt-get install -y --no-install-recommends perl make pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

# Build without the default `local-onnx` feature. That feature pulls in
# fastembed -> ort -> onnxruntime (a large C++ static lib that fails to link
# in a clean Debian toolchain). The MCP server does not need local ONNX
# embeddings to serve or introspect its tools, so dropping it keeps the image
# small and buildable while `remem mcp` stays fully functional for Glama.
RUN cargo build --release --bin remem --no-default-features \
    && strip target/release/remem

# --- runtime stage -----------------------------------------------------------
FROM debian:bookworm-slim

# ca-certificates for outbound HTTPS (LLM extraction/summarization calls).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 remem

COPY --from=builder /src/target/release/remem /usr/local/bin/remem

USER remem
WORKDIR /home/remem

# The MCP server runs a database preflight before serving tools and refuses to
# open an unencrypted database unless explicitly allowed. This image is an
# ephemeral introspection/demo sandbox with no real memory data, so a plaintext
# throwaway DB is fine. For persistent real use, mount an encrypted DB and run
# `remem encrypt` instead of relying on this flag.
ENV REMEM_ALLOW_PLAINTEXT_DB=1

# `remem mcp` speaks the MCP protocol over stdio; keep STDIN open with -i.
ENTRYPOINT ["remem", "mcp"]
