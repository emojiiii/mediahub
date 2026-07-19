FROM rust:1.88-bookworm AS builder
ARG LIBVIPS_VERSION=8.18.4
ARG LIBVIPS_SHA256=1e8d2228a4ffae7498e357dcddb3775440afa7b11726841cd511674dced84257
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
COPY . .
RUN cargo build --release --package mediahub-server --features docker-libvips

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        libexpat1 \
        libglib2.0-0 \
        libjpeg62-turbo \
        libpng16-16 \
        libwebp7 \
        libwebpdemux2 \
        libwebpmux3 \
        zlib1g \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 mediahub
WORKDIR /app
COPY --from=builder /opt/libvips/lib/ /usr/local/lib/
COPY --from=builder /app/target/release/mediahub-server /usr/local/bin/mediahub-server
RUN ldconfig
USER mediahub
ENV MEDIAHUB_BIND_ADDR=0.0.0.0:3000
ENV MEDIAHUB_DATABASE_URL=postgres://mediahub:mediahub-local-only@postgres:5432/mediahub
ENV MEDIAHUB_STORAGE_ROOT=/data/storage
VOLUME ["/data"]
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/mediahub-server"]
