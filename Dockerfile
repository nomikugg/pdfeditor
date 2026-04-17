# syntax=docker/dockerfile:1

FROM rust:1.85-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/pdf-editor-backend /app/pdf-editor-backend
COPY libpdfium.so /app/libpdfium.so
RUN mkdir -p /app/files \
    && chmod 777 /app/files

ENV PDFIUM_LIBRARY_PATH=/app/libpdfium.so
ENV FILES_ROOT=/app/files
ENV BIND_HOST=0.0.0.0
ENV PORT=8080

EXPOSE 8080

CMD ["/app/pdf-editor-backend"]
