# syntax=docker/dockerfile:1

FROM rust:1.90-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release

COPY src ./src
RUN cargo build --release


FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
         libc6 \
         libgcc-s1 \
       libstdc++6 \
       fontconfig \
         libfreetype6 \
         libjpeg62-turbo \
         libpng16-16 \
         libopenjp2-7 \
         liblcms2-2 \
         libnss3 \
         libexpat1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/pdf-editor-backend /app/app
COPY libpdfium.so /app/libpdfium.so
COPY src/fonts /app/src/fonts

RUN mkdir -p /app/files \
    && chmod 777 /app/files

ENV PDFIUM_LIBRARY_PATH=/app/libpdfium.so
ENV FILES_ROOT=/app/files
ENV BIND_HOST=0.0.0.0

EXPOSE 8080

CMD ["/app/app"]