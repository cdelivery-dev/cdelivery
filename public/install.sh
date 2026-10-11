#!/bin/sh
set -eu

say() {
    printf '%s\n' "$*" >&2
}

die() {
    say "Error (${stage:-preflight}): $*"
    exit 1
}

privilege() (
    runtime_uid=$(id -u) || die "Cannot determine the current user."
    if [ "$runtime_uid" -eq 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo -- "$@"
    elif command -v doas >/dev/null 2>&1; then
        doas -- "$@"
    else
        die "Root access is required for this operation; run as root or use sudo/doas."
    fi
)

runtime_require() {
    for runtime_command in "$@"; do
        command -v "$runtime_command" >/dev/null 2>&1 || die "Missing required command: $runtime_command."
    done
}

runtime_detect() {
    ID=
    VERSION_ID=
    NAME=
    runtime_family=
    runtime_suite=
    if [ -r /etc/os-release ]; then
        . /etc/os-release
    fi
    case "$ID:$VERSION_ID" in
        ubuntu:22.04) runtime_family=apt; runtime_suite=jammy ;;
        ubuntu:24.04) runtime_family=apt; runtime_suite=noble ;;
        ubuntu:26.04) runtime_family=apt; runtime_suite=resolute ;;
        debian:12) runtime_family=apt; runtime_suite=bookworm ;;
        debian:13) runtime_family=apt; runtime_suite=trixie ;;
        fedora:43|fedora:44|rhel:8|rhel:8.*|rhel:9|rhel:9.*|rhel:10|rhel:10.*) runtime_family=rpm ;;
        centos:9|centos:10)
            if [ "$NAME" = 'CentOS Stream' ]; then runtime_family=rpm; fi ;;
        alpine:*) runtime_family=apk ;;
    esac
}

runtime_apk_repository() (
    runtime_branch=$(printf '%s\n' "$VERSION_ID" | awk '
        /^[0-9]+\.[0-9]+(\.[0-9]+)?$/ {
            split($0, version, ".")
            print "v" version[1] "." version[2]
            valid = 1
        }
        END { exit !valid }
    ') || die "Automatic package installation requires a stable Alpine release."
    [ -r /etc/apk/repositories ] || die "Cannot read /etc/apk/repositories."
    runtime_community=$(awk -v branch="$runtime_branch" '
        /^[[:space:]]*(#|$)/ { next }
        {
            repository = $1
            if (repository ~ /^@/) repository = $2
            sub(/\/+$/, "", repository)
            count = split(repository, segments, "/")
            release = segments[count - 1]
            if (release == "edge" || (release ~ /^v[0-9]+\.[0-9]+$/ && release != branch)) invalid = 1
            if ($1 !~ /^@/ && release == branch) {
                if (segments[count] == "community") community = 1
                if (segments[count] == "main" && repository ~ /^https?:\/\// && main == "") {
                    sub(/\/main$/, "/community", repository)
                    main = repository
                }
            }
        }
        END {
            if (invalid || (!community && main == "")) exit 1
            if (!community) print main
        }
    ' /etc/apk/repositories) || die "Alpine repositories must use $runtime_branch without mixed release branches."
    if [ -n "$runtime_community" ]; then
        say "Enabling the Alpine $runtime_branch community repository."
        printf '\n%s\n' "$runtime_community" | privilege tee -a /etc/apk/repositories >/dev/null || die "Cannot enable the Alpine community repository."
    fi
)

install_dependencies() (
    if [ "$host_os" = Darwin ]; then
        runtime_require curl
        [ -x /usr/bin/plutil ] || die "This macOS installation requires /usr/bin/plutil."
        exit 0
    fi
    runtime_missing=
    for runtime_command in curl jq ip; do
        command -v "$runtime_command" >/dev/null 2>&1 || runtime_missing="$runtime_missing $runtime_command"
    done
    runtime_ca=
    for runtime_bundle in "${CURL_CA_BUNDLE:-}" "${SSL_CERT_FILE:-}" /etc/ssl/certs/ca-certificates.crt /etc/pki/tls/certs/ca-bundle.crt /etc/ssl/ca-bundle.pem /etc/ssl/cert.pem; do
        if [ -r "$runtime_bundle" ] && [ -s "$runtime_bundle" ]; then runtime_ca=yes; break; fi
    done
    [ -n "$runtime_ca" ] || runtime_missing="$runtime_missing ca-certificates"
    [ -n "$runtime_missing" ] || exit 0
    runtime_detect
    [ -n "$runtime_family" ] || die "Missing dependencies:$runtime_missing. Install them using this system's package manager, then rerun the installer."
    say "Installing required dependencies:$runtime_missing."
    case "$runtime_family" in
        apt)
            runtime_require apt-get
            privilege apt-get update || die "Cannot update APT package indexes."
            privilege env DEBIAN_FRONTEND=noninteractive apt-get -y --no-remove --no-install-recommends install curl ca-certificates jq iproute2 || die "Cannot install required dependencies." ;;
        rpm)
            runtime_require dnf
            set -- ca-certificates jq iproute
            command -v curl >/dev/null 2>&1 || set -- curl "$@"
            privilege dnf -y --setopt=install_weak_deps=False install "$@" || die "Cannot install required dependencies." ;;
        apk)
            runtime_require apk awk tee
            runtime_apk_repository || exit 1
            privilege apk add --no-cache curl ca-certificates jq iproute2 || die "Cannot install required dependencies." ;;
    esac
)

runtime_install_file() {
    if [ -e "$2" ] || [ -L "$2" ]; then
        cmp -s "$1" "$2" || die "Existing repository configuration differs: $2. Configure Docker manually."
    else
        privilege install -m 0644 "$1" "$2" || die "Cannot install repository configuration: $2."
    fi
}

runtime_wait_docker() (
    runtime_deadline=$(( $(date +%s) + 30 ))
    while [ "$(date +%s)" -lt "$runtime_deadline" ]; do
        if docker_endpoint=unix:///var/run/docker.sock docker_privileged=yes docker_cmd info >/dev/null 2>&1; then exit 0; fi
        sleep 1
    done
    die "Docker did not become ready within 30 seconds. Inspect the Docker service logs."
)

mac_runtime_hint() {
    if ! command -v brew >/dev/null 2>&1; then say "Homebrew is available at https://brew.sh."; fi
    say "To prepare OrbStack:"
    say "  brew install --cask orbstack"
    say "  orb start"
    say "If Docker Desktop is already installed, start it instead."
}

install_docker() (
    command -v docker >/dev/null 2>&1 && exit 0
    if [ "$host_os" = Darwin ]; then
        mac_runtime_hint
        die "A running OrbStack or Docker Desktop installation is required."
    fi
    if command -v dockerd >/dev/null 2>&1 || [ -S /var/run/docker.sock ]; then
        die "A Docker daemon exists without its CLI. Install the matching Docker CLI before continuing."
    fi
    runtime_detect
    [ -n "$runtime_family" ] || die "Automatic Docker installation is unavailable for $ID $VERSION_ID. Install Docker, then rerun the installer."
    case "$(uname -m)" in
        x86_64|aarch64|arm64) ;;
        *) die "CDelivery requires an amd64 or arm64 Docker daemon." ;;
    esac
    case "$runtime_family" in
        apt)
            runtime_require apt-get dpkg dpkg-query systemctl install cmp awk
            [ -d /run/systemd/system ] || die "Automatic Docker installation requires a running systemd on $ID."
            for runtime_package in docker.io docker-compose docker-compose-v2 docker-doc docker-buildx podman-docker containerd runc; do
                if dpkg-query -W -f='${Status}\n' "$runtime_package" 2>/dev/null |
                    awk '$0 == "install ok installed" { found = 1 } END { exit !found }'; then
                    die "Conflicting package $runtime_package is installed. Resolve it before installing Docker."
                fi
            done ;;
        rpm)
            runtime_require dnf rpm systemctl install cmp
            [ -d /run/systemd/system ] || die "Automatic Docker installation requires a running systemd on $ID."
            for runtime_package in docker docker-client docker-client-latest docker-common docker-latest docker-latest-logrotate docker-logrotate docker-selinux docker-engine-selinux docker-engine podman-docker containerd runc; do
                if rpm -q "$runtime_package" >/dev/null 2>&1; then
                    die "Conflicting package $runtime_package is installed. Resolve it before installing Docker."
                fi
            done
            if [ "$ID" = rhel ] && rpm -q podman >/dev/null 2>&1; then
                die "Conflicting package podman is installed. Resolve it before installing Docker."
            fi ;;
        apk)
            runtime_require apk rc-service rc-update awk tee
            [ -d /run/openrc ] || die "Automatic Docker installation on Alpine requires a running OpenRC."
            if apk info -e podman-docker >/dev/null 2>&1; then
                die "Conflicting package podman-docker is installed. Resolve it before installing Docker."
            fi ;;
    esac
    install_dependencies || exit 1
    say "Installing Docker on $NAME $VERSION_ID."
    if [ "$runtime_family" = apk ]; then
        runtime_apk_repository || exit 1
        privilege apk add --no-cache docker-engine docker-cli docker-openrc || die "Cannot install Docker packages."
        privilege rc-update add docker default || die "Cannot enable Docker at startup."
        privilege rc-service docker start || die "Cannot start Docker; inspect /var/log/docker.log and rc-service docker status."
        runtime_wait_docker
        exit 0
    fi
    if [ "$runtime_family" = apt ]; then
        runtime_arch=$(dpkg --print-architecture) || die "Cannot determine the package architecture."
        curl --proto '=https' --proto-redir '=https' -fsSL --connect-timeout 15 --max-time 60 \
            "https://download.docker.com/linux/$ID/gpg" -o "$workdir/docker.asc" || die "Cannot download the Docker signing key."
        printf '%s\n' 'Types: deb' "URIs: https://download.docker.com/linux/$ID" \
            "Suites: $runtime_suite" 'Components: stable' "Architectures: $runtime_arch" \
            'Signed-By: /etc/apt/keyrings/docker.asc' > "$workdir/docker.sources" || die "Cannot prepare the Docker repository."
        privilege install -d -m 0755 /etc/apt/keyrings /etc/apt/sources.list.d || die "Cannot create APT repository directories."
        runtime_install_file "$workdir/docker.asc" /etc/apt/keyrings/docker.asc
        runtime_install_file "$workdir/docker.sources" /etc/apt/sources.list.d/docker.sources
        privilege apt-get update || die "Cannot load the Docker APT repository."
        privilege env DEBIAN_FRONTEND=noninteractive apt-get -y --no-remove --no-install-recommends install docker-ce docker-ce-cli containerd.io || die "Cannot install Docker packages."
    else
        curl --proto '=https' --proto-redir '=https' -fsSL --connect-timeout 15 --max-time 60 \
            "https://download.docker.com/linux/$ID/docker-ce.repo" -o "$workdir/docker-ce.repo" || die "Cannot download the Docker repository."
        runtime_install_file "$workdir/docker-ce.repo" /etc/yum.repos.d/docker-ce.repo
        privilege dnf -y --setopt=gpgcheck=True --setopt=install_weak_deps=False install docker-ce docker-ce-cli containerd.io || die "Cannot install Docker packages."
    fi
    privilege systemctl enable --now docker || die "Cannot start Docker; inspect systemctl status docker."
    runtime_wait_docker
)

usage() {
    cat <<'EOF'
Usage: install.sh [--advertise-addr IPv4] [--help]

Install CDelivery using the current release catalog.
--advertise-addr IPv4  Use this server communication address without a menu.
                      Use a stable address reachable by your other servers.
                      127.0.0.1 is for a single server only.
--help                Show this help.

An existing installation is checked without upgrading or replacing it.
EOF
}

valid_address() {
    case "$1" in
        ''|*[!0-9.]*) return 1 ;;
    esac
    printf '%s\n' "$1" | awk -F. '
        NF != 4 { exit 1 }
        {
            for (i = 1; i <= 4; i++)
                if ($i !~ /^[0-9]+$/ || $i > 255 || (length($i) > 1 && substr($i, 1, 1) == "0")) exit 1
            if ($0 == "127.0.0.1") exit 0
            if ($1 == 0 || $1 == 127 || $1 >= 224 || ($1 == 169 && $2 == 254)) exit 1
        }
    '
}

docker_cmd() (
    unset DOCKER_CONTEXT DOCKER_HOST DOCKER_TLS_VERIFY DOCKER_CERT_PATH DOCKER_API_VERSION
    if [ "$docker_privileged" = yes ]; then
        privilege docker --host "$docker_endpoint" "$@"
    else
        docker --host "$docker_endpoint" "$@"
    fi
)

connect_docker() {
    docker_privileged=no
    if [ -n "${DOCKER_CONTEXT:-}" ]; then
        docker_endpoint=$(docker context inspect "$DOCKER_CONTEXT" --format '{{.Endpoints.docker.Host}}') || die "Cannot inspect the selected Docker context."
    elif [ -n "${DOCKER_HOST:-}" ]; then
        docker_endpoint=$DOCKER_HOST
    else
        docker_context=$(docker context show) || die "Cannot determine the Docker context."
        docker_endpoint=$(docker context inspect "$docker_context" --format '{{.Endpoints.docker.Host}}') || die "Cannot inspect the Docker context."
    fi
    case "$docker_endpoint" in
        unix:///*) ;;
        *) die "A local Docker Unix socket is required; the selected endpoint is remote or unsupported." ;;
    esac
    if [ "$host_os" = Linux ]; then
        case "$docker_endpoint" in
            unix:///var/run/docker.sock|unix:///run/docker.sock) ;;
            *) die "Native Linux Docker must use /var/run/docker.sock or /run/docker.sock." ;;
        esac
    fi
    if ! docker_cmd info --format '{{.OSType}}' > "$workdir/docker-os" 2> "$workdir/docker-error"; then
        docker_socket=${docker_endpoint#unix://}
        if [ "$host_os" = Linux ] && [ -S "$docker_socket" ] && { [ ! -r "$docker_socket" ] || [ ! -w "$docker_socket" ]; }; then
            docker_privileged=yes
            docker_cmd info --format '{{.OSType}}' > "$workdir/docker-os" || die "Cannot access the selected Docker daemon with elevated privileges."
        else
            cat "$workdir/docker-error" >&2
            if [ "$host_os" = Darwin ]; then
                mac_runtime_hint
            fi
            die "The selected Docker daemon is not ready."
        fi
    fi
    [ "$(cat "$workdir/docker-os")" = linux ] || die "A Linux Docker daemon is required."
    docker_arch=$(docker_cmd info --format '{{.Architecture}}') || die "Cannot read the daemon architecture."
    case "$docker_arch" in
        amd64|x86_64|arm64|aarch64) ;;
        *) die "The Docker daemon must use amd64 or arm64." ;;
    esac
    docker_security=$(docker_cmd info --format '{{range .SecurityOptions}}{{println .}}{{end}}') || die "Cannot read Docker security options."
    case "$docker_security" in
        *rootless*) die "A rootful Docker daemon is required." ;;
    esac
    if [ "$host_os" = Darwin ]; then
        docker_runtime=$(docker_cmd info --format '{{.OperatingSystem}}') || die "Cannot identify the Docker runtime."
        case "$docker_runtime" in
            *OrbStack*|*Docker\ Desktop*) ;;
            *) die "Use OrbStack or Docker Desktop on macOS." ;;
        esac
    fi
    docker_versions=$(docker_cmd version --format '{{.Server.MinAPIVersion}} {{.Server.APIVersion}}') || die "Cannot read the Docker API range."
    printf '%s\n' "$docker_versions" | awk '
        function number(v, parts) {
            if (v !~ /^[0-9]+\.[0-9]+$/) return -1
            split(v, parts, ".")
            return parts[1] * 1000 + parts[2]
        }
        { low = number($1); high = number($2); exit !(NF == 2 && low >= 0 && low <= 1048 && high >= 1048) }
    ' || die "The Docker API range ($docker_versions) does not include 1.48."
    say "Using $docker_endpoint ($docker_arch)."
}

cluster_state() {
    cluster=$(docker_cmd info --format '{{.Swarm.LocalNodeState}}') || die "Cannot read the cluster state."
    case "$cluster" in
        inactive) return ;;
        active)
            [ "$(docker_cmd info --format '{{.Swarm.ControlAvailable}}')" = true ] || die "This server is a worker; run the installer on a manager."
            existing_endpoint=$(docker_cmd node inspect self --format '{{.ManagerStatus.Addr}}') || die "Cannot read the manager address."
            existing_address=${existing_endpoint%:*}
            case "$existing_address" in
                \[*\]) existing_address=${existing_address#\[}; existing_address=${existing_address%\]} ;;
            esac
            if [ -n "$advertise_addr" ] && [ "$advertise_addr" != "$existing_address" ]; then
                die "The existing manager uses $existing_endpoint; --advertise-addr cannot change an existing cluster."
            fi
            advertise_addr=$existing_address
            say "Keeping the existing server communication address: $existing_endpoint."
            ;;
        *) die "The cluster is in state '$cluster'; resolve it before installing." ;;
    esac
}

network_id() {
    docker_cmd network inspect cdelivery --format '{{if and (eq .Driver "overlay") (eq .Scope "swarm")}}{{.Id}}{{end}}'
}

verify_service() {
    actual_network=$(network_id) || die "The installed service has no valid cdelivery network."
    [ -n "$actual_network" ] || die "The cdelivery network must be an overlay network."
    docker_cmd service inspect cdelivery --format '{{if .Spec.Mode.Replicated}}{{.Spec.Mode.Replicated.Replicas}}{{else}}invalid{{end}}
{{range .Spec.TaskTemplate.Placement.Constraints}}{{println .}}{{end}}
{{range .Spec.TaskTemplate.Networks}}{{println .Target}}{{end}}
{{range .Spec.TaskTemplate.ContainerSpec.Mounts}}{{if and (eq .Type "bind") (eq .Source "/var/run/docker.sock") (eq .Target "/var/run/docker.sock") (not .ReadOnly)}}socket{{end}}{{end}}
{{range .Spec.EndpointSpec.Ports}}{{if and (eq .Protocol "tcp") (eq .PublishedPort 9000) (eq .TargetPort 9000) (eq .PublishMode "ingress")}}port{{end}}{{end}}' > "$workdir/service" || die "Cannot inspect the installed service."
    awk -v network="$actual_network" '
        NR == 1 && $0 != "1" { exit 1 }
        /^[[:space:]]*node\.role[[:space:]]*==[[:space:]]*manager[[:space:]]*$/ { manager = 1 }
        $0 == network || $0 == "cdelivery" { attached = 1 }
        $0 == "socket" { socket = 1 }
        $0 == "port" { port = 1 }
        END { if (!(manager && attached && socket && port)) exit 1 }
    ' "$workdir/service" || die "The existing cdelivery service does not match the required single-manager deployment. No configuration was changed."
}

existing_installation() {
    [ "$cluster" = active ] || return 1
    for resource in service secret config network; do
        docker_cmd "$resource" ls --format '{{.Name}} {{.ID}}' > "$workdir/resources" || die "Cannot list existing $resource resources."
        if [ "$resource" = service ] && grep -q '^cdelivery ' "$workdir/resources"; then
            stage='existing installation'
            verify_service
            command -v curl >/dev/null 2>&1 || die "Install curl to check the existing dashboard."
            wait_ready
            say "CDelivery is already installed. Its image and configuration were preserved."
            show_dashboard
            return 0
        fi
        case "$resource" in
            network) labels_template='{{range $key, $value := .Labels}}{{println $key}}{{end}}' ;;
            *) labels_template='{{range $key, $value := .Spec.Labels}}{{println $key}}{{end}}' ;;
        esac
        while IFS=' ' read -r resource_name resource_ref; do
            case "$resource:$resource_name" in
                network:cdelivery) continue ;;
                *:cdelivery|*:cdelivery-*) die "CDelivery $resource $resource_name exists without the main service. Restore the installation before continuing." ;;
            esac
            if [ "$resource" != network ]; then resource_ref=$resource_name; fi
            docker_cmd "$resource" inspect "$resource_ref" --format "$labels_template" > "$workdir/labels" || die "Cannot inspect $resource $resource_name labels."
            if grep -q '^dev\.cdelivery\.' "$workdir/labels"; then
                die "Platform-owned $resource $resource_name exists without the main service. Restore the installation before continuing."
            fi
        done < "$workdir/resources"
    done
    return 1
}

discover_addresses() {
    if [ "$host_os" = Linux ]; then
        ip -o -4 address show up > "$workdir/interfaces" || die "Cannot discover IPv4 addresses."
        awk '$3 == "inet" {
            split($4, address, "/")
            sub(/@.*/, "", $2)
            print address[1], $2, address[2]
        }' "$workdir/interfaces" > "$workdir/addresses"
    else
        ifconfig -a > "$workdir/interfaces" || die "Cannot discover IPv4 addresses."
        awk '
            function prefix(mask, value, i, digit, bits) {
                sub(/^0x/, "", mask)
                value = 0
                for (i = 1; i <= length(mask); i++) {
                    digit = index("0123456789abcdef", tolower(substr(mask, i, 1))) - 1
                    value = value * 16 + digit
                }
                while (value > 0) { bits += value % 2; value = int(value / 2) }
                return bits + 0
            }
            function flush(i) {
                if (up && !inactive && !(members > 0 && members == virtual))
                    for (i = 1; i <= count; i++) print addresses[i], interface, prefixes[i]
                count = members = virtual = inactive = 0
            }
            /^[^[:space:]]/ {
                flush()
                interface = $1; sub(/:$/, "", interface)
                up = $0 ~ /[<,]UP[,>]/
            }
            $1 == "inet" {
                addresses[++count] = $2
                prefixes[count] = 32
                for (i = 3; i < NF; i++) if ($i == "netmask") prefixes[count] = prefix($(i + 1))
            }
            $1 == "member:" { members++; if ($2 ~ /^vmenet/) virtual++ }
            $1 == "status:" && $2 == "inactive" { inactive = 1 }
            END { flush() }
        ' "$workdir/interfaces" > "$workdir/addresses"
    fi
    awk '
        {
            split($1, octets, ".")
            if (octets[1] == 0 || octets[1] == 127 || octets[1] >= 224 || (octets[1] == 169 && octets[2] == 254)) next
            if ($2 ~ /^(docker[0-9]+|docker_gwbridge|veth.*|br-[0-9a-f]+|virbr[0-9]+|vmenet[0-9]+|vboxnet[0-9]+|vmnet[0-9]+)$/) next
            private = octets[1] == 10 || (octets[1] == 172 && octets[2] >= 16 && octets[2] <= 31) || (octets[1] == 192 && octets[2] == 168)
            shared = octets[1] == 100 && octets[2] >= 64 && octets[2] <= 127
            vpn = $2 ~ /^(tun|tap|utun|wg|ppp|tailscale)/
            type = private ? "private" : (shared ? "shared" : "public")
            if (vpn) type = type "/VPN"
            print (private ? 1 : 2), $1, $2, type, $3
        }
    ' "$workdir/addresses" > "$workdir/candidates-unsorted"
    sort -k1,1n -k3,3 -k2,2 "$workdir/candidates-unsorted" > "$workdir/candidates-sorted"
    awk '!seen[$2]++' "$workdir/candidates-sorted" > "$workdir/candidates"
}

choose_address() {
    discover_addresses
    if [ -n "$advertise_addr" ]; then
        say "Using server communication address: $advertise_addr."
        return
    fi
    printf '%s\n' '3 127.0.0.1 loopback single-server-only 32' >> "$workdir/candidates"
    say "Select a stable server communication address reachable by your other servers:"
    awk '{ printf "  %d) %s  %s  [%s]\n", NR, $2, $3, $4 }' "$workdir/candidates" >&2
    if ! (exec 3<> /dev/tty) 2>/dev/null; then
        die "Selection needs a terminal. Rerun with --advertise-addr IPv4."
    fi
    exec 3<> /dev/tty
    while :; do
        printf 'Address number: ' >&3
        IFS= read -r selection <&3 || die "Address selection ended."
        case "$selection" in
            ''|*[!0-9]*) say "Enter one of the listed numbers."; continue ;;
        esac
        advertise_addr=$(awk -v choice="$selection" 'NR == choice { print $2 }' "$workdir/candidates")
        [ -n "$advertise_addr" ] && break
        say "Enter one of the listed numbers."
    done
    exec 3>&-
    say "Using server communication address: $advertise_addr."
}

fetch_image() {
    curl --proto '=https' --proto-redir '=https' -fsSL --connect-timeout 10 --max-time 30 --max-filesize 131072 \
        "${releases_url}v1/ghcr.io/cdelivery-dev/latest.json" -o "$workdir/releases.json" || die "Cannot download the release catalog."
    [ "$(wc -c < "$workdir/releases.json")" -le 131072 ] || die "The release catalog exceeds 128 KiB."
    if [ "$host_os" = Darwin ]; then
        plutil -extract cdelivery.image raw -expect string -n -o "$workdir/image" "$workdir/releases.json" || die "The release catalog has no valid cdelivery.image string."
    else
        jq -serj 'select(length == 1) | .[0].cdelivery.image | select(type == "string")' "$workdir/releases.json" > "$workdir/image" || die "The release catalog has no valid cdelivery.image string."
    fi
    invalid_characters=$(LC_ALL=C tr -d 'a-zA-Z0-9/.:_-' < "$workdir/image" | wc -c)
    [ "$invalid_characters" -eq 0 ] || die "The release image contains invalid characters."
    image=$(cat "$workdir/image")
    printf '%s\n' "$image" | awk '
        /^ghcr\.io\/cdelivery-dev\/cdelivery:[vV]?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/ { valid = 1 }
        END { exit !valid }
    ' || die "The release image must use the CDelivery repository and a stable SemVer tag."
    say "Pulling $image."
    docker_cmd pull "$image" || die "Cannot pull $image. Check registry access and architecture support."
    docker_cmd image inspect "$image" --format '{{range .RepoDigests}}{{println .}}{{end}}' > "$workdir/digests" || die "Cannot inspect the pulled image."
    image_digest=$(awk '
        /^ghcr\.io\/cdelivery-dev\/cdelivery@sha256:[0-9a-f]+$/ {
            split($0, parts, "@")
            if (length(parts[2]) == 71) { digest = parts[2]; count++ }
        }
        END { if (count != 1) exit 1; print digest }
    ' "$workdir/digests") || die "The pulled image has no unique repository digest."
    image="$image@$image_digest"
}

check_port() {
    if [ "$cluster" = active ]; then
        docker_cmd service ls --format '{{.Ports}}' > "$workdir/ports" || die "Cannot inspect published service ports."
        if grep -Eq ':9000->[0-9]+/tcp' "$workdir/ports"; then
            die "Port 9000 is already published by another service."
        fi
    fi
    docker_cmd ps -q > "$workdir/container-ids" || die "Cannot list running containers."
    while IFS= read -r container_id; do
        docker_cmd container inspect "$container_id" --format '{{range $port, $bindings := .NetworkSettings.Ports}}{{range $bindings}}{{printf "%s %s\n" $port .HostPort}}{{end}}{{end}}' > "$workdir/container-ports" || die "Cannot inspect container ports."
        if awk '$1 ~ /\/tcp$/ && $2 == "9000" { found = 1 } END { exit !found }' "$workdir/container-ports"; then
            die "TCP port 9000 is already published by a container."
        fi
    done < "$workdir/container-ids"
    if [ "$host_os" = Darwin ]; then
        netstat -an -p tcp > "$workdir/listeners" || die "Cannot inspect host TCP ports."
        if awk '$6 == "LISTEN" && $4 ~ /[.:]9000$/ { found = 1 } END { exit !found }' "$workdir/listeners"; then
            die "TCP port 9000 is already in use."
        fi
    else
        [ -r /proc/net/tcp ] || die "Cannot inspect host TCP ports."
        awk '$4 == "0A" && toupper($2) ~ /:2328$/ { found = 1 } END { exit found }' /proc/net/tcp || die "TCP port 9000 is already in use."
        if [ -r /proc/net/tcp6 ]; then
            awk '$4 == "0A" && toupper($2) ~ /:2328$/ { found = 1 } END { exit found }' /proc/net/tcp6 || die "TCP port 9000 is already in use."
        fi
    fi
}

check_network() {
    docker_cmd network ls --format '{{.Name}}' > "$workdir/network-names" || die "Cannot inspect networks."
    if grep -qx cdelivery "$workdir/network-names"; then
        network_owner=$(docker_cmd network inspect cdelivery --format '{{index .Labels "dev.cdelivery.installer"}}') || die "Cannot inspect the existing cdelivery network."
        [ "$network_owner" = true ] || die "The name cdelivery is occupied by a network not created by this installer."
        [ -n "$(network_id)" ] || die "The existing cdelivery network is not a cluster overlay."
    fi
}

check_subnets() {
    docker_cmd network inspect ingress cdelivery --format '{{range .IPAM.Config}}{{println .Subnet}}{{end}}' > "$workdir/subnets" || die "Cannot inspect cluster subnets."
    awk '{ print $2 "/" $5 }' "$workdir/candidates-unsorted" > "$workdir/underlay"
    printf '%s/32\n' "$advertise_addr" >> "$workdir/underlay"
    awk '
        function start(cidr, segments, octets, n) {
            split(cidr, segments, "/")
            split(segments[1], octets, ".")
            size = 2 ^ (32 - segments[2])
            n = ((octets[1] * 256 + octets[2]) * 256 + octets[3]) * 256 + octets[4]
            return int(n / size) * size
        }
        NR == FNR {
            if ($0 ~ /^[0-9.]+\/[0-9]+$/) { low[++count] = start($0); high[count] = low[count] + size - 1 }
            next
        }
        /^[0-9.]+\/[0-9]+$/ {
            a = start($0); b = a + size - 1
            for (i = 1; i <= count; i++)
                if (a <= high[i] && low[i] <= b) { print "Conflicting cluster subnet: " $0; conflict = 1 }
        }
        END { exit conflict }
    ' "$workdir/underlay" "$workdir/subnets" >&2 || die "Cluster subnets overlap a server network. Resolve the network configuration before continuing."
}

deploy() {
    if [ "$cluster" = inactive ]; then
        docker_cmd swarm init --advertise-addr "$advertise_addr" --listen-addr 0.0.0.0:2377 > "$workdir/init" || die "Cannot initialize the cluster with $advertise_addr."
        cluster=active
    fi
    check_network
    if ! grep -qx cdelivery "$workdir/network-names"; then
        docker_cmd network create --driver overlay --label dev.cdelivery.installer=true cdelivery > /dev/null || die "Cannot create the cdelivery network."
    fi
    check_subnets
    docker_cmd service create --detach --no-resolve-image --with-registry-auth --name cdelivery --label dev.cdelivery.installer=true \
        --mode replicated --replicas 1 --constraint node.role==manager --update-order stop-first \
        --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock \
        --network cdelivery --publish published=9000,target=9000,protocol=tcp,mode=ingress \
        --env "CDELIVERY_RELEASES_URL=$releases_url" "$image" > /dev/null || die "Cannot create the cdelivery service."
}

wait_ready() {
    ready_deadline=$(( $(date +%s) + 180 ))
    while [ "$(date +%s)" -lt "$ready_deadline" ]; do
        docker_cmd service ps cdelivery --filter desired-state=running --format '{{.CurrentState}}' > "$workdir/tasks" || die "Cannot inspect service tasks."
        if grep -q '^Running ' "$workdir/tasks"; then
            http_result=$(curl --noproxy '*' -sS --connect-timeout 2 --max-time 3 -o /dev/null -w '%{http_code} %{redirect_url}' http://127.0.0.1:9000/setup 2>/dev/null) || http_result=000
            case "$http_result" in
                '200 '*|'303 http://127.0.0.1:9000/login') return ;;
            esac
        fi
        sleep 2
    done
    docker_cmd service ps cdelivery --no-trunc --format '{{.Name}}: {{.CurrentState}} {{.Error}}' >&2 || :
    die "CDelivery did not become ready within 180 seconds. Existing resources were retained."
}

show_dashboard() {
    if [ "$host_os" = Darwin ]; then
        dashboard_host=localhost
    else
        dashboard_host=$advertise_addr
        case "$dashboard_host" in
            *:*) dashboard_host="[$dashboard_host]" ;;
        esac
    fi
    say "Open http://$dashboard_host:9000 to set up or sign in to CDelivery."
}

main() {
    advertise_addr=
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --help) usage; return ;;
            --advertise-addr)
                [ "$#" -ge 2 ] && [ -z "$advertise_addr" ] || die "Provide --advertise-addr exactly once with an IPv4 address."
                advertise_addr=$2
                valid_address "$advertise_addr" || die "--advertise-addr requires a valid unicast IPv4 address or 127.0.0.1."
                shift
                ;;
            *) die "Unknown argument: $1. Use --help." ;;
        esac
        shift
    done
    host_os=$(uname -s)
    case "$host_os" in
        Linux|Darwin) ;;
        *) die "Only Linux and macOS are supported." ;;
    esac
    umask 077
    workdir=$(mktemp -d "${TMPDIR:-/tmp}/cdelivery-install.XXXXXX") || die "Cannot create a temporary directory."
    trap 'rm -rf "$workdir"' 0
    trap 'exit 130' INT
    trap 'exit 143' HUP TERM
    releases_url=https://releases.cdelivery.dev/
    stage='Docker preparation'
    install_docker
    connect_docker
    stage='existing resources'
    cluster_state
    if existing_installation; then return; fi
    stage=dependencies
    install_dependencies
    stage='address selection'
    choose_address
    stage=preflight
    check_port
    check_network
    stage='image download'
    fetch_image
    stage=deployment
    deploy
    stage='service readiness'
    say "Waiting for CDelivery to become ready (up to 180 seconds)."
    wait_ready
    say "CDelivery is ready."
    show_dashboard
}

main "$@"
