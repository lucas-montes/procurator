# Multi-stage build for Rust workspace
FROM rust:1.75-alpine as builder

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev pkgconfig postgresql-dev curl

WORKDIR /app

# Copy workspace manifests and build dependencies first (caching)
COPY Cargo.toml Cargo.lock ./
COPY crates/myapp-core/Cargo.toml ./crates/myapp-core/Cargo.toml
COPY crates/myapp-api/Cargo.toml ./crates/myapp-api/Cargo.toml
COPY crates/myapp-cli/Cargo.toml ./crates/myapp-cli/Cargo.toml

# Create dummy source files to build dependencies
RUN mkdir -p crates/myapp-core/src && echo "pub fn dummy() {}" > crates/myapp-core/src/lib.rs && \
    mkdir -p crates/myapp-api/src && echo "fn main() {}" > crates/myapp-api/src/main.rs && \
    mkdir -p crates/myapp-cli/src && echo "fn main() {}" > crates/myapp-cli/src/main.rs && \
    cargo build --release && \
    rm -rf crates/*/src

# Copy actual source code
COPY crates ./crates
RUN cargo build --release --bin myapp-api

# Runtime stage
FROM alpine:3.18

# Runtime dependencies
RUN apk add --no-cache libgcc openssl ca-certificates libpq

RUN addgroup -g 1000 app && adduser -D -u 1000 -G app app

WORKDIR /app
COPY --from=builder /app/target/release/myapp-api /app/myapp-api
RUN chown -R app:app /app

USER app

ENV RUST_LOG=info \
    DATABASE_URL=postgres://localhost/myapp \
    PORT=8080

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD curl -f http://localhost:8080/health || exit 1

CMD ["/app/myapp-api"]
