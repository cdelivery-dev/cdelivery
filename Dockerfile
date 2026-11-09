FROM --platform=linux/amd64 alpine:3.23 AS tini-amd64
RUN apk add --no-cache gcc musl-dev cmake make patch
COPY tini /src/tini
RUN cmake -S /src/tini -B /build && cmake --build /build && test "$(wc -c < /build/tini)" -lt 512000

FROM --platform=linux/arm64 alpine:3.23 AS tini-arm64
RUN apk add --no-cache gcc musl-dev cmake make patch
COPY tini /src/tini
RUN cmake -S /src/tini -B /build && cmake --build /build && test "$(wc -c < /build/tini)" -lt 512000

FROM scratch AS tini-bundle
COPY --from=tini-amd64 /build/tini /amd64
COPY --from=tini-arm64 /build/tini /arm64

FROM rust:slim-trixie AS build

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install \
       --yes --no-install-recommends \
       cmake g++ git jq libclang-common-19-dev libclang1-19 make perl util-linux

WORKDIR /src

RUN --mount=type=cache,id=cdelivery-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cdelivery-cargo-git,target=/usr/local/cargo/git \
    cargo install --locked --features cli --root /opt/license-tools cargo-about

ENV PATH="/opt/license-tools/bin:${PATH}"

ARG BIN_NAME
ARG TARGETARCH

COPY . .

ARG PROFILE

RUN --mount=type=cache,id=cdelivery-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cdelivery-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=cdelivery-cargo-target-${TARGETARCH},target=/src/target,sharing=locked \
    test -n "$BIN_NAME" \
    && test -n "$PROFILE" \
    && cargo build --locked --profile "$PROFILE" --package "$BIN_NAME" --bin "$BIN_NAME" \
    && case "$TARGETARCH" in amd64) target=x86_64-unknown-linux-gnu ;; arm64) target=aarch64-unknown-linux-gnu ;; *) exit 1 ;; esac \
    && bash scripts/licenses.sh "$BIN_NAME" "$target" \
    && profile_dir="$PROFILE" \
    && if [ "$PROFILE" = dev ]; then profile_dir=debug; fi \
    && mkdir -p /out/app \
        && mkdir -p /out/usr/share/licenses/cdelivery \
    && cp "target/licenses/$BIN_NAME/$target/LICENSE" "target/licenses/$BIN_NAME/$target/NOTICE" "target/licenses/$BIN_NAME/$target/THIRD_PARTY_LICENSES.txt" "target/licenses/$BIN_NAME/$target/RUST_STDLIB_LICENSES.html" /out/usr/share/licenses/cdelivery/ \
    && cp "target/$profile_dir/$BIN_NAME" "/out/app/$BIN_NAME" \
    && ln -s "/app/$BIN_NAME" /out/entrypoint

RUN --mount=from=tini-bundle,target=/tmp/tini \
    if [ "$BIN_NAME" = cdelivery ]; then \
        mkdir -p /out/usr/share/cdelivery/tini /out/usr/share/licenses/cdelivery/tini \
        && cp /tmp/tini/amd64 /tmp/tini/arm64 /out/usr/share/cdelivery/tini/ \
        && cp tini/vendor/tini/LICENSE /out/usr/share/licenses/cdelivery/tini/LICENSE; \
    fi

RUN if [ "$BIN_NAME" = cdelivery-agent ]; then \
        mkdir -p /out/usr/bin /out/usr/share/licenses/cdelivery/util-linux \
        && cp /usr/bin/nsenter /out/usr/bin/ \
        && cp /usr/share/doc/util-linux/copyright /out/usr/share/licenses/cdelivery/util-linux/ \
        && ldd /usr/bin/nsenter | awk '/=> \// {print $3} /^[[:space:]]*\// {print $1}' | \
           while IFS= read -r library; do \
               directory="/out$(dirname "$(readlink -f "$library")")" \
               && mkdir -p "$directory" \
               && cp -L "$library" "$directory/$(basename "$library")" || exit; \
           done \
        && for package in libselinux1 libpcre2-8-0; do \
               mkdir -p "/out/usr/share/licenses/cdelivery/$package" \
               && cp "/usr/share/doc/$package/copyright" "/out/usr/share/licenses/cdelivery/$package/" || exit; \
           done; \
    fi

FROM gcr.io/distroless/cc-debian13:latest

LABEL org.opencontainers.image.source="https://github.com/cdelivery-dev/cdelivery"
LABEL org.opencontainers.image.licenses="Apache-2.0"

COPY --from=build /out/ /
ENTRYPOINT ["/entrypoint"]
