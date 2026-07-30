FROM rust:1.75-bookworm AS builder

WORKDIR /usr/src/bee

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    cmake \
    clang \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY bee ./bee
COPY plugins ./plugins

RUN cargo build --release \
    -p bee \
    -p bee-plugin-onnx-ml \
    -p bee-plugin-perf-fib \
    -p bee-plugin-sample-kline

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    iputils-ping \
    net-tools \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /usr/src/bee/target/release/bee /usr/local/bin/bee

RUN mkdir -p /etc/bee/plugins /var/log/bee

ENV BEE_PLUGIN_DIR=/etc/bee/plugins
ENV RUST_LOG=info

EXPOSE 7701 8701

ENTRYPOINT ["/usr/local/bin/bee"]
CMD ["node", "--help"]