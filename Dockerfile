FROM node:22-bookworm-slim AS web-builder
WORKDIR /app/web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY openapi /app/openapi
COPY web ./
RUN pnpm build

FROM rust:1.97.0-bookworm AS builder
ARG LIBVIPS_VERSION=8.18.4
ARG LIBVIPS_SHA256=1e8d2228a4ffae7498e357dcddb3775440afa7b11726841cd511674dced84257
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        libexpat1-dev \
        libglib2.0-dev \
        libjpeg62-turbo-dev \
        libpng-dev \
        libwebp-dev \
        meson \
        ninja-build \
        pkg-config \
        zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*
RUN curl --fail --silent --show-error --location \
        "https://github.com/libvips/libvips/archive/refs/tags/v${LIBVIPS_VERSION}.tar.gz" \
        --output /tmp/libvips.tar.gz \
    && echo "${LIBVIPS_SHA256}  /tmp/libvips.tar.gz" | sha256sum --check --strict \
    && tar --extract --gzip --file /tmp/libvips.tar.gz --directory /tmp \
    && meson setup /tmp/libvips-build "/tmp/libvips-${LIBVIPS_VERSION}" \
        --buildtype=release \
        --prefix=/opt/libvips \
        --libdir=lib \
        --wrap-mode=nodownload \
        -Dmodules=disabled \
        -Dexamples=false \
        -Dcplusplus=false \
        -Dintrospection=disabled \
        -Dcfitsio=disabled \
        -Dcgif=disabled \
        -Dexif=disabled \
        -Dfftw=disabled \
        -Dfontconfig=disabled \
        -Dheif=disabled \
        -Dimagequant=disabled \
        -Djpeg=enabled \
        -Djpeg-xl=disabled \
        -Dlcms=disabled \
        -Dmagick=disabled \
        -Dmatio=disabled \
        -Dnifti=disabled \
        -Dopenexr=disabled \
        -Dopenjpeg=disabled \
        -Dopenslide=disabled \
        -Dorc=disabled \
        -Dpangocairo=disabled \
        -Dpdfium=disabled \
        -Dpng=enabled \
        -Dpoppler=disabled \
        -Dquantizr=disabled \
        -Drsvg=disabled \
        -Dspng=disabled \
        -Dtiff=disabled \
        -Dwebp=enabled \
        -Dzlib=enabled \
        -Dnsgif=false \
        -Dppm=false \
        -Danalyze=false \
        -Dradiance=false \
    && meson compile -C /tmp/libvips-build \
    && meson install -C /tmp/libvips-build \
    && rm -rf /tmp/libvips.tar.gz /tmp/libvips-build "/tmp/libvips-${LIBVIPS_VERSION}"
ENV PKG_CONFIG_PATH=/opt/libvips/lib/pkgconfig
ENV LD_LIBRARY_PATH=/opt/libvips/lib
ENV LIBRARY_PATH=/opt/libvips/lib
WORKDIR /app

# Keep dependency compilation independent from application source changes.
COPY Cargo.toml Cargo.lock ./
COPY crates/mediahub-adapter-image/Cargo.toml crates/mediahub-adapter-image/Cargo.toml
COPY crates/mediahub-adapter-local/Cargo.toml crates/mediahub-adapter-local/Cargo.toml
COPY crates/mediahub-adapter-postgres/Cargo.toml crates/mediahub-adapter-postgres/Cargo.toml
COPY crates/mediahub-adapter-s3/Cargo.toml crates/mediahub-adapter-s3/Cargo.toml
COPY crates/mediahub-app/Cargo.toml crates/mediahub-app/Cargo.toml
COPY crates/mediahub-core/Cargo.toml crates/mediahub-core/Cargo.toml
COPY crates/mediahub-openapi/Cargo.toml crates/mediahub-openapi/Cargo.toml
COPY crates/mediahub-server/Cargo.toml crates/mediahub-server/Cargo.toml
RUN for crate in \
        mediahub-adapter-image \
        mediahub-adapter-local \
        mediahub-adapter-postgres \
        mediahub-adapter-s3 \
        mediahub-app \
        mediahub-core \
        mediahub-openapi \
        mediahub-server; do \
        mkdir --parents "crates/${crate}/src"; \
        printf 'pub fn dependency_placeholder() {}\n' > "crates/${crate}/src/lib.rs"; \
    done \
    && printf 'fn main() {}\n' > crates/mediahub-openapi/src/main.rs \
    && printf 'fn main() {}\n' > crates/mediahub-server/src/main.rs
RUN cargo build --release --package mediahub-server --features docker-libvips

COPY crates ./crates
RUN for crate in \
        mediahub-adapter-image \
        mediahub-adapter-local \
        mediahub-adapter-postgres \
        mediahub-adapter-s3 \
        mediahub-app \
        mediahub-core \
        mediahub-openapi \
        mediahub-server; do \
        cargo clean --release --package "${crate}"; \
    done \
    && cargo build --release --package mediahub-server --features docker-libvips

FROM debian:bookworm-slim AS runtime
LABEL org.opencontainers.image.title="MediaHub" \
      org.opencontainers.image.description="Self-hosted media object storage and processing service" \
      org.opencontainers.image.source="https://github.com/emojiiii/mediahub" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        libexpat1 \
        libglib2.0-0 \
        libjpeg62-turbo \
        libpng16-16 \
        libwebp7 \
        libwebpdemux2 \
        libwebpmux3 \
        zlib1g \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 mediahub \
    && install --directory --owner=mediahub --group=mediahub --mode=0750 /data/storage
WORKDIR /app
COPY --from=builder /opt/libvips/lib/ /usr/local/lib/
COPY --from=builder /app/target/release/mediahub-server /usr/local/bin/mediahub-server
COPY --from=web-builder --chown=mediahub:mediahub /app/web/dist /app/web
RUN ldconfig
USER mediahub
ENV MEDIAHUB_BIND_ADDR=0.0.0.0:3000
ENV MEDIAHUB_DATABASE_URL=postgres://mediahub:mediahub-local-only@postgres:5432/mediahub
ENV MEDIAHUB_STORAGE_ROOT=/data/storage
ENV MEDIAHUB_WEB_ROOT=/app/web
VOLUME ["/data"]
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:3000/health/live"]
ENTRYPOINT ["/usr/local/bin/mediahub-server"]
