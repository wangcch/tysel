# syntax=docker/dockerfile:1
ARG TYSEL_VERSION
ARG TYSEL_TOOLCHAIN_IMAGE=ghcr.io/wangcch/tysel-toolchain:${TYSEL_VERSION}
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian13:nonroot

FROM ${TYSEL_TOOLCHAIN_IMAGE} AS build
WORKDIR /src
COPY . .

RUN tysel task verify \
    --manifest tysel.container.toml
RUN tysel build --release \
    --manifest tysel.container.toml \
    --output /out/tysel-app

FROM ${RUNTIME_IMAGE}
# CI may add version, source, revision, and admitted artifact digest labels with
# docker build --label. Do not substitute the registry image digest for the
# executable digest from the verified .sha256 sidecar or `tysel image` output.
WORKDIR /app
COPY --from=build --chown=65532:65532 /out/tysel-app /app/tysel-app
USER 65532:65532
EXPOSE 3000
ENTRYPOINT ["/app/tysel-app"]
