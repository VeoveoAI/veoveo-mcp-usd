# syntax=docker/dockerfile:1.27@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e
# The factory environment: the toolchain that builds a branch in its OpenShell
# sandbox. The installation mounts the Lore CLI that fetches it.
FROM rust:1.99.0-trixie@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates clang cmake git pkg-config tzdata \
    && rm -rf /var/lib/apt/lists/* /var/log/apt /var/log/dpkg.log \
    && rustup component add clippy rustfmt
# The persistent disk keeps fetched crates across restarts; cargo fetches pinned git
# dependencies with git, which the policy admits.
ENV CARGO_HOME=/sandbox/.cargo CARGO_NET_GIT_FETCH_WITH_CLI=true
# OpenShell runs the workload as uid 10001.
RUN groupadd --gid 10001 sandbox \
    && useradd --create-home --uid 10001 --gid 10001 --shell /bin/bash sandbox \
    && chown -R sandbox:sandbox /usr/local/rustup /usr/local/cargo \
    && mkdir -p /sandbox && chown sandbox:sandbox /sandbox
USER sandbox
WORKDIR /sandbox
