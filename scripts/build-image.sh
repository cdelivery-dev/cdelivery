#!/bin/sh
set -eu

usage() {
    printf 'usage: %s <cdelivery|cdelivery-agent|pingress|insight|insight-agent> [--release] [--platform <amd64|arm64|multi>] [--push]\n' "$0" >&2
    exit 2
}

[ "$#" -ge 1 ] || usage

target=$1
shift
profile=dev
platform=
push=false
while [ "$#" -gt 0 ]; do
    case $1 in
        --release) profile=release-speed ;;
        --push) push=true ;;
        --platform)
            [ "$#" -ge 2 ] && [ -z "$platform" ] || usage
            shift
            case $1 in
                amd64 | arm64 | multi) platform=$1 ;;
                *)
                    printf 'unknown platform: %s\n' "$1" >&2
                    exit 2
                    ;;
            esac
            ;;
        *) usage ;;
    esac
    shift
done

case $target in
    cdelivery) [ "$profile" = dev ] || profile=release-size ;;
    cdelivery-agent | pingress | insight | insight-agent) ;;
    *)
        printf 'unknown target: %s\n' "$target" >&2
        exit 2
        ;;
esac

CDPATH= cd -- "$(dirname -- "$0")/.."
package_id=$(cargo pkgid --locked --package "$target")
version=${package_id##*#}
image="ghcr.io/cdelivery-dev/${target}:${version}"

build() {
    set -- --tag "$image" "$@"
    case $platform in
        multi) set -- --platform linux/amd64,linux/arm64 "$@" ;;
        amd64 | arm64) set -- --platform "linux/$platform" "$@" ;;
    esac
    if [ "$push" = true ]; then
        exec docker buildx build --push "$@"
    fi
    if [ "$platform" != multi ]; then
        exec docker buildx build --load "$@"
    fi

    archive="target/images/${target}-${version}-linux-amd64-arm64.oci.tar"
    mkdir -p target/images
    staging=$(mktemp "${archive}.tmp.XXXXXX")
    trap 'rm -f -- "$staging"' 0
    trap 'exit 1' HUP INT TERM

    docker buildx build \
        --output "type=oci,name=$image,dest=$staging" \
        "$@"
    mv -f -- "$staging" "$archive"
    trap - 0 HUP INT TERM
    printf '%s\n' "$archive"
}

build \
    --build-arg BIN_NAME="$target" \
    --build-arg PROFILE="$profile" \
    .
