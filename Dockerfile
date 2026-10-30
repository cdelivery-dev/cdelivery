FROM rust:slim-trixie AS build

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install \
       --yes --no-install-recommends \
       cmake g++ git jq libclang-common-19-dev libclang1-19 make perl

WORKDIR /src

RUN <<EOF
[ -d "$CARGO_HOME" ]
cat > "$CARGO_HOME/config.toml" <<'CONFIG'
[source.crates-io]
replace-with = "rsproxy-sparse"
[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
CONFIG
EOF

RUN --mount=type=cache,id=cdelivery-cargo-home,target=/tmp/cargo-home \
    cp /usr/local/cargo/config.toml /tmp/cargo-home/config.toml \
    && CARGO_HOME=/tmp/cargo-home cargo install --locked --version 0.9.3 --features cli --root /opt/license-tools cargo-about

ENV PATH="/opt/license-tools/bin:${PATH}"

ARG BIN_NAME
ARG TARGETARCH

COPY . .

ARG PROFILE

RUN --mount=type=cache,id=cdelivery-cargo-home,target=/tmp/cargo-home \
    --mount=type=cache,id=cdelivery-cargo-target-${TARGETARCH},target=/src/target,sharing=locked \
    test -n "$BIN_NAME" \
    && test -n "$PROFILE" \
    && cp /usr/local/cargo/config.toml /tmp/cargo-home/config.toml \
    && CARGO_HOME=/tmp/cargo-home cargo build --locked --profile "$PROFILE" --package "$BIN_NAME" --bin "$BIN_NAME" \
    && case "$TARGETARCH" in amd64) target=x86_64-unknown-linux-gnu ;; arm64) target=aarch64-unknown-linux-gnu ;; *) exit 1 ;; esac \
    && CARGO_HOME=/tmp/cargo-home bash scripts/licenses.sh "$BIN_NAME" "$target" \
    && profile_dir="$PROFILE" \
    && if [ "$PROFILE" = dev ]; then profile_dir=debug; fi \
    && mkdir -p /out/app \
    && mkdir -p /out/usr/share/licenses/cdelivery \
    && cp "target/licenses/$BIN_NAME/$target/LICENSE" "target/licenses/$BIN_NAME/$target/NOTICE" "target/licenses/$BIN_NAME/$target/THIRD_PARTY_LICENSES.txt" "target/licenses/$BIN_NAME/$target/RUST_STDLIB_LICENSES.html" /out/usr/share/licenses/cdelivery/ \
    && cp "target/$profile_dir/$BIN_NAME" "/out/app/$BIN_NAME" \
    && ln -s "/app/$BIN_NAME" /out/entrypoint

FROM gcr.io/distroless/cc-debian13:latest

LABEL org.opencontainers.image.source="https://github.com/cdelivery-dev/cdelivery"
LABEL org.opencontainers.image.licenses="Apache-2.0"

COPY --from=build /out/ /
ENTRYPOINT ["/entrypoint"]
