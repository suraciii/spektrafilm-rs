FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential pkg-config curl ca-certificates git python3 xz-utils unzip \
    libgtk-3-dev libudev-dev libxkbcommon-dev liblensfun-dev libexiv2-dev \
    libglib2.0-dev libopenimageio-dev libjpeg-dev zlib1g-dev libopenblas-dev \
    libvulkan1 mesa-vulkan-drivers vulkan-tools \
    && mkdir -p /opt/verification \
    && dpkg-query -W > /opt/verification/packages.tsv \
    && rm -rf /var/lib/apt/lists/*

# Tools live outside HOME, which Actions overrides for container jobs.
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:${PATH}
RUN curl -fsSL --retry 3 -o /tmp/rustup-init https://static.rust-lang.org/rustup/archive/1.28.2/x86_64-unknown-linux-gnu/rustup-init \
    && echo '20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c  /tmp/rustup-init' | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain 1.99.1 --component rustfmt --component clippy \
    && rm /tmp/rustup-init
RUN curl -fsSL --retry 3 -o /tmp/just.tar.gz https://github.com/casey/just/releases/download/1.45.0/just-1.45.0-x86_64-unknown-linux-musl.tar.gz \
    && echo 'dc3f958aaf8c6506dd90426e9b03f86dd15e74a6467ee0e54929f750af3d9e49  /tmp/just.tar.gz' | sha256sum -c - \
    && tar -xzf /tmp/just.tar.gz -C /usr/local/bin just && rm /tmp/just.tar.gz
RUN curl -fsSL --retry 3 -o /tmp/LibRaw.tar.gz https://www.libraw.org/data/LibRaw-0.22.0.tar.gz \
    && echo '1071e6e8011593c366ffdadc3d3513f57c90202d526e133174945ec1dd53f2a1  /tmp/LibRaw.tar.gz' | sha256sum -c - \
    && tar -xzf /tmp/LibRaw.tar.gz -C /tmp \
    && cd /tmp/LibRaw-0.22.0 && ./configure --prefix=/opt/libraw --disable-static --disable-examples \
    && make -j2 && make install && rm -rf /tmp/LibRaw*
ENV PKG_CONFIG_PATH=/opt/libraw/lib/pkgconfig LD_LIBRARY_PATH=/opt/libraw/lib
# Normalize distro-specific ICD filenames once, at image construction.
RUN icd=$(find /usr/share/vulkan/icd.d -name 'lvp_icd*.json' -print -quit) \
    && test -n "$icd" && cp "$icd" /opt/verification/lvp.json \
    && mkdir -m 700 /tmp/verification-runtime \
    && { rustc -Vv; cargo -V; just --version; python3 --version; pkg-config --modversion libraw; } > /opt/verification/tools.txt
ENV VK_ICD_FILENAMES=/opt/verification/lvp.json XDG_RUNTIME_DIR=/tmp/verification-runtime
ENV OPENBLAS_NUM_THREADS=1 RAYON_NUM_THREADS=2 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
CMD ["bash"]
