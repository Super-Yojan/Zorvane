FROM rust:1-bookworm AS build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config build-essential cmake git ca-certificates clang lld \
    libwayland-dev libxkbcommon-dev libvulkan-dev libxcursor-dev \
    libxrandr-dev libxi-dev libx11-xcb-dev libasound2-dev libudev-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .
ENV CARGO_BUILD_JOBS=2
RUN cargo build --release --locked -p zorvane

FROM debian:bookworm-slim AS runtime

ENV DEBIAN_FRONTEND=noninteractive \
    ZORVANE_ASSETS=/usr/local/share/zorvane/assets \
    TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447 \
    WGPU_BACKEND=vulkan \
    LIBGL_ALWAYS_SOFTWARE=1

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libwayland-client0 libxkbcommon0 libx11-xcb1 libxcursor1 \
    libxrandr2 libxi6 libx11-6 libxext6 libxfixes3 libxrender1 \
    libxcb1 libxkbcommon-x11-0 libasound2 libudev1 \
    libvulkan1 mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/zorvane /usr/local/bin/zorvane
COPY --from=build /src/simulator/assets /usr/local/share/zorvane/assets

EXPOSE 7447
ENTRYPOINT ["/usr/local/bin/zorvane"]
