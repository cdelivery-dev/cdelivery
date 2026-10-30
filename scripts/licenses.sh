#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf 'usage: %s <cdelivery|cdelivery-agent|pingress|insight|insight-agent> <x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu>\n' "$0" >&2
    exit "${1:-2}"
}

case ${1:-} in -h | --help) usage 0 ;; esac
[ "$#" -eq 2 ] || usage
package=$1
target=$2
case $package in cdelivery | cdelivery-agent | pingress | insight | insight-agent) ;; *) usage ;; esac
case $target in x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu) ;; *) usage ;; esac

CDPATH= cd -- "$(dirname -- "$0")/.."
if [ "$(cargo-about --version)" != 'cargo-about 0.9.3' ]; then
    printf 'Install cargo-about 0.9.3; see licenses/README.md\n' >&2
    exit 1
fi
output="target/licenses/$package/$target"
mkdir -p "$output"
staging=$(mktemp -d "$output/.licenses.XXXXXX")
trap 'rm -rf -- "$staging"' EXIT
trap 'exit 1' HUP INT TERM

cargo-about generate --locked --format json --fail \
    --config about.toml --manifest-path "$package/Cargo.toml" \
    --target "$target" --output-file "$output/dependencies.json"

jq -j '.crates[] | select(.package.source != null) |
    (.package.name + "@" + .package.version), "\u0000",
    .license, "\u0000", .package.manifest_path, "\u0000"
' "$output/dependencies.json" > "$staging/packages"
: > "$staging/texts.json"
printf 'Third-party licenses for %s (%s)\nIncludes upstream license alternatives and notices. These do not change\nCDelivery\047s Apache-2.0 license. Dependency expressions follow Cargo.lock.\n\n' \
    "$package" "$target" > "$staging/inventory"

append_file() {
    local notice=false
    case ${1##*/} in [Nn][Oo][Tt][Ii][Cc][Ee]*) notice=true ;; esac
    jq -n --rawfile text "$1" --arg label "$2" --argjson notice "$notice" '
        ($text | gsub("\r\n?"; "\n")) as $text |
        if $text | test("\\S") then {text: $text, label: $label, notice: $notice}
        else error("Empty license: " + $label) end
    ' >> "$staging/texts.json"
}

while IFS= read -r -d '' name && IFS= read -r -d '' expression && IFS= read -r -d '' manifest; do
    printf '%s: %s\n' "$name" "$expression" >> "$staging/inventory"
    directory=${manifest%/*}
    find "$directory" -type f \( -iname 'LICENSE*' -o -iname 'LICENCE*' \
        -o -iname 'COPYING*' -o -iname 'COPYRIGHT*' -o -iname 'NOTICE*' \) \
        -print0 > "$staging/files"
    found=false
    while IFS= read -r -d '' path; do
        append_file "$path" "$name: ${path#"$directory"/}"
        found=true
    done < "$staging/files"
    jq -j --arg name "$name" '.[] | select(.packages | index($name)) |
        ("licenses/" + .file), "\u0000",
        ($name + ": " + .source + "\n" + (.notice // "")), "\u0000"
    ' licenses/supplements.json > "$staging/supplements"
    while IFS= read -r -d '' path && IFS= read -r -d '' label; do
        append_file "$path" "$label"
        found=true
    done < "$staging/supplements"
    if [ "$found" = false ]; then
        printf 'No upstream license for %s; review licenses/supplements.json\n' "$name" >&2
        exit 1
    fi
done < "$staging/packages"

cp "$staging/inventory" "$output/THIRD_PARTY_LICENSES.txt"
cp NOTICE "$output/NOTICE"
printf '\n' >> "$output/NOTICE"
for file in THIRD_PARTY_LICENSES.txt NOTICE; do
    jq -sr --arg file "$file" '
        map(select($file != "NOTICE" or .notice)) | group_by(.text)[] |
        "\n" + ("=" * 80) + "\n" + (map(.label) | unique | join("\n")) + "\n\n" + .[0].text
    ' "$staging/texts.json" >> "$output/$file"
done
if [ "$package" = cdelivery ]; then
    printf '\n%080d\n' 0 | tr 0 = >> "$output/THIRD_PARTY_LICENSES.txt"
    cat cdelivery/assets/THIRD_PARTY_LICENSES.txt >> "$output/THIRD_PARTY_LICENSES.txt"
    printf '\n' >> "$output/THIRD_PARTY_LICENSES.txt"
fi
cp LICENSE "$output/LICENSE"
sysroot=$(rustc --print sysroot)
cp "$sysroot/share/doc/rust/COPYRIGHT-library.html" "$output/RUST_STDLIB_LICENSES.html"
printf 'licenses: %s %s\n' "$package" "$target"
