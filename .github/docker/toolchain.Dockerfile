# syntax=docker/dockerfile:1
ARG TOOLCHAIN_BASE_IMAGE=debian:13.3-slim@sha256:1d3c811171a08a5adaa4a163fbafd96b61b87aa871bbc7aa15431ac275d3d430
FROM ${TOOLCHAIN_BASE_IMAGE}

ARG TYSEL_VERSION
ARG TYSEL_SOURCE_COMMIT
ARG TYSEL_SOURCE_URL=https://github.com/wangcch/tysel

COPY bin/ /opt/tysel/bin/
COPY LICENSE /opt/tysel/LICENSE

ENV PATH="/opt/tysel/bin:${PATH}"
WORKDIR /workspace

LABEL org.opencontainers.image.title="Tysel toolchain" \
      org.opencontainers.image.description="Tysel CLI, service stub, and isolated worker for Linux application builds" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="${TYSEL_SOURCE_URL}" \
      org.opencontainers.image.revision="${TYSEL_SOURCE_COMMIT}" \
      org.opencontainers.image.version="${TYSEL_VERSION}"

ENTRYPOINT ["tysel"]
CMD ["--help"]
