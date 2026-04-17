# syntax=docker/dockerfile:1

FROM rust:1.90-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release \
    && ls -lh /app/target/release/pdf-editor-backend


FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
       libstdc++6 \
       fontconfig \
       libfreetype6 \
    libjpeg62-turbo \
       libpng16-16 \
       libopenjp2-7 \
       liblcms2-2 \
       libnss3 \
       libexpat1 \
       zlib1g \
         libx11-6 \
         libx11-xcb1 \
         libxcb1 \
         libxext6 \
         libxrender1 \
         libxfixes3 \
         libxdamage1 \
         libxrandr2 \
         libxcomposite1 \
         libxkbcommon0 \
         libdrm2 \
         libgbm1 \
         libglib2.0-0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/pdf-editor-backend /app/app
COPY libpdfium.so /app/libpdfium.so
COPY src/fonts /app/src/fonts

RUN mkdir -p /app/files \
    && chmod 777 /app/files

ENV PDFIUM_LIBRARY_PATH=/app/libpdfium.so
ENV FILES_ROOT=/app/files
ENV BIND_HOST=0.0.0.0
ENV RUST_BACKTRACE=1
ENV STARTUP_DEBUG=1

EXPOSE 8080

CMD ["/bin/sh", "-c", "set -x; echo '== runtime config =='; echo \"PORT=${PORT:-unset}\"; echo \"BIND_HOST=${BIND_HOST:-unset}\"; echo \"PDFIUM_LIBRARY_PATH=${PDFIUM_LIBRARY_PATH:-unset}\"; echo \"FILES_ROOT=${FILES_ROOT:-unset}\"; echo \"STARTUP_DEBUG=${STARTUP_DEBUG:-unset}\"; echo '== app binary =='; ls -l /app/app; ldd /app/app || true; echo '== libpdfium diagnostics =='; ls -l /app/libpdfium.so; ldd /app/libpdfium.so || true; echo '== app start =='; /app/app; code=$?; echo \"== app exit code: ${code} ==\"; exit ${code}"]