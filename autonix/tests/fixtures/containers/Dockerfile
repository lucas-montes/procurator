# Multi-stage Dockerfile for Rust application
FROM rust:1.75-alpine as builder

# Install system dependencies
RUN apk add --no-cache musl-dev openssl-dev pkgconfig curl

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Build dependencies (cached layer)
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy source code
COPY src ./src

# Build application
RUN cargo build --release

# Runtime stage
FROM alpine:3.18

# Install runtime dependencies
RUN apk add --no-cache libgcc openssl ca-certificates

# Create app user
RUN addgroup -g 1000 app && \
    adduser -D -u 1000 -G app app

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/myapp /app/myapp

# Set ownership
RUN chown -R app:app /app

USER app

# Environment variables
ENV RUST_LOG=info \
    APP_PORT=8080 \
    DATABASE_URL=postgres://localhost/db

EXPOSE 8080 9090

HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
  CMD curl -f http://localhost:8080/health || exit 1

LABEL version="1.0.0" \
      maintainer="team@example.com" \
      description="Rust application"

CMD ["/app/myapp"]
