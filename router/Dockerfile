# zenohd 1.10.1 plus the remote-api plugin the browser simulator needs.
# The official eclipse/zenoh image does not ship that plugin.

FROM debian:bookworm-slim AS plugin
ARG TARGETARCH
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip \
    && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) triple=x86_64-unknown-linux-musl ;; \
      arm64) triple=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/plugin.zip \
      "https://github.com/eclipse-zenoh/zenoh-ts/releases/download/1.10.1/zenoh-ts-1.10.1-${triple}-standalone.zip"; \
    mkdir -p /out; \
    unzip -j /tmp/plugin.zip "libzenoh_plugin_remote_api.so" -d /out

FROM eclipse/zenoh:1.10.1
COPY --from=plugin /out/libzenoh_plugin_remote_api.so /zenoh-plugins/libzenoh_plugin_remote_api.so
COPY zenoh.json5 /etc/zenoh/zenoh.json5
EXPOSE 7447 10000
CMD ["-c", "/etc/zenoh/zenoh.json5"]
