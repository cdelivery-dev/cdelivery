#!/bin/sh
set -eu

catalog=https://releases.cdelivery.dev/v1/ghcr.io/cdelivery-dev/latest.json
api=1.48

say() {
    printf '%s\n' "$*" >&2
}

die() {
    say "Error: $*"
    exit 1
}

usage() {
    cat <<'EOF'
Usage: install.sh [--advertise-addr IPv4] [--no-ipv6]
       install.sh --join URL --token INVITATION [--advertise-addr IPv4] [--no-ipv6]

Install CDelivery, or join this server to an existing CDelivery instance.

  --advertise-addr IPv4  Use this server communication address without a menu.
                         Your other servers must be able to reach it.
                         127.0.0.1 is for a single server only.
  --join URL             Join the CDelivery instance at this HTTP(S) origin.
  --token INVITATION     Use its join invitation.
  --no-ipv6              Keep an IPv4-only gateway. Pingress then sees IPv6
                         clients as the gateway address.
  --help                 Show this help.

Rerunning on an existing installation only checks it. If a step fails,
resolve the reported problem and rerun the installer.
EOF
}

root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    elif command -v doas >/dev/null 2>&1; then
        doas "$@"
    else
        die "Root access is required; run as root or install sudo."
    fi
}

ask() {
    printf '%s' "$1" > /dev/tty && IFS= read -r answer < /dev/tty
} 2>/dev/null

retry() {
    tries=$1 delay=$2
    shift 2
    until "$@"; do
        tries=$((tries - 1))
        [ "$tries" -gt 0 ] || return 1
        sleep "$delay"
    done
}

as_docker() {
    if [ "$docker_root" = yes ]; then root "$@"; else "$@"; fi
}

dk() {
    as_docker docker -H "unix://$socket" "$@"
}

install_docker() {
    ID='' VERSION_ID='' VERSION_CODENAME=''
    [ ! -r /etc/os-release ] || . /etc/os-release
    repo=https://download.docker.com/linux/$ID
    unavailable="Docker packages are unavailable for ${ID:-this system} ${VERSION_CODENAME:-$VERSION_ID}. Install Docker 28 or newer, then rerun."
    say "Installing Docker."
    case "$ID" in
        ubuntu|debian)
            curl -fsSI "$repo/dists/$VERSION_CODENAME/Release" >/dev/null || die "$unavailable"
            root install -d -m 0755 /etc/apt/keyrings
            root curl -fsSL "$repo/gpg" -o /etc/apt/keyrings/docker.asc
            root chmod a+r /etc/apt/keyrings/docker.asc
            printf 'Types: deb\nURIs: %s\nSuites: %s\nComponents: stable\nArchitectures: %s\nSigned-By: /etc/apt/keyrings/docker.asc\n' \
                "$repo" "$VERSION_CODENAME" "$(dpkg --print-architecture)" | root tee /etc/apt/sources.list.d/docker.sources >/dev/null
            root apt-get update
            root env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-remove docker-ce docker-ce-cli containerd.io
            ;;
        fedora|rhel|centos)
            curl -fsSI "$repo/${VERSION_ID%%.*}/$(uname -m)/stable/repodata/repomd.xml" >/dev/null || die "$unavailable"
            root curl -fsSL "$repo/docker-ce.repo" -o /etc/yum.repos.d/docker-ce.repo
            root dnf install -y docker-ce docker-ce-cli containerd.io
            ;;
        alpine)
            root apk add docker || die "Cannot install Docker. Enable the Alpine community repository, then rerun."
            root rc-update add docker default
            root rc-service docker start
            ;;
        *) die "$unavailable" ;;
    esac
    [ "$ID" = alpine ] || root systemctl enable --now docker
    docker_wait=30
}

docker_ready() {
    docker_root=no
    docker -H "unix://$socket" info >/dev/null 2>&1 && return 0
    [ "$os" = Linux ] && docker_root=yes && root docker -H "unix://$socket" info >/dev/null 2>&1
}

connect_docker() {
    if [ "$os" = Linux ]; then
        socket=/var/run/docker.sock
    else
        endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
        case "$endpoint" in
            unix://*) socket=${endpoint#unix://} ;;
            *) die "The current Docker context uses $endpoint; a local Docker socket is required." ;;
        esac
    fi
    retry "$docker_wait" 1 docker_ready || die "Cannot reach Docker at $socket. Start Docker, then rerun."
    version=$(dk version --format '{{.Server.APIVersion}}')
    printf '%s\n' "$version" | awk -F. -v min="$api" '{ split(min, m, "."); exit !($1 > m[1] || ($1 == m[1] && $2 >= m[2])) }' ||
        die "Docker API $version is too old. Upgrade to Docker 28 or newer, then rerun."
}

addresses() {
    if [ "$os" = Linux ]; then
        ip -o -4 address show | awk '{ sub(/@.*/, "", $2); sub(/\/.*/, "", $4); print $4, $2 }'
    else
        ifconfig | awk '/^[^ \t]/ { name = $1; sub(/:$/, "", name); up = /[<,]UP[,>]/ } up && $1 == "inet" { print $2, name }'
    fi | awk '
        $1 ~ /^(127|169\.254)\./ || $2 ~ /^(lo|docker|br-|veth|virbr|bridge|vmenet|vboxnet|vmnet)/ { next }
        {
            split($1, o, ".")
            rank = o[1] == 10 || (o[1] == 172 && o[2] >= 16 && o[2] <= 31) || (o[1] == 192 && o[2] == 168) ? 1 : 2
            print rank, $1, "(" $2 ")"
        }
    ' | sort -n | awk '!seen[$2]++ { print $2, $3 }'
}

choose_addr() {
    [ -z "$addr" ] || return 0
    list=$(addresses; [ -n "$join_url" ] || echo '127.0.0.1 (single server only)')
    [ -n "$list" ] || die "No usable IPv4 address found. Rerun with --advertise-addr IPv4."
    say "Select a stable server communication address that your other servers can reach:"
    printf '%s\n' "$list" | awk '{ printf "  %d) %s\n", NR, $0 }' >&2
    while :; do
        ask 'Address number: ' || die "Address selection needs a terminal. Rerun with --advertise-addr IPv4."
        addr=$(printf '%s\n' "$list" | awk -v choice="$answer" 'NR == choice { print $1 }')
        [ -z "$addr" ] || break
        say "Enter one of the listed numbers."
    done
}

check_port() {
    if [ "$os" = Linux ]; then
        cat /proc/net/tcp /proc/net/tcp6 2>/dev/null | awk '$4 == "0A" && $2 ~ /:2328$/ { exit 1 }'
    else
        ! lsof -nP -iTCP:9000 -sTCP:LISTEN >/dev/null
    fi || die "TCP port 9000 is already in use."
}

release_image() {
    image=$(curl -fsSL --max-time 30 --max-filesize 131072 "$catalog" | tr -d ' \t\r\n' | awk '{
        s = substr($0, index($0, "\"cdelivery\":{"))
        if (match(s, /"image":"[^"]*"/)) print substr(s, RSTART + 9, RLENGTH - 10)
    }')
    printf '%s\n' "$image" | grep -Eqx 'ghcr\.io/cdelivery-dev/cdelivery:v?[0-9]+\.[0-9]+\.[0-9]+' ||
        die "Cannot read a valid cdelivery image from the release catalog."
}

ipv6_decline() {
    say "$1"
    say "Without an IPv6 gateway, Pingress sees IPv6 clients as the gateway's IPv4 address."
    ask '  1) Continue with IPv4 only.
  2) Exit, resolve it, then rerun (default).
Choice [2]: ' || answer=
    [ "$answer" = 1 ] || die "IPv6 gateway preparation stopped. Resolve it and rerun, or rerun with --no-ipv6 to continue with IPv4 only."
    ipv6=skipped
}

ra_prepare() {
    for dev in $( {
        awk '$4 == "00" && substr($5, 1, 1) ~ /[0-7]/ { print $6 }' /proc/net/if_inet6
        ip -6 route show default 2>/dev/null | awk '/ proto (ra|9)( |$)/ { for (i = 1; i < NF; i++) if ($i == "dev") print $(i + 1) }'
    } | sort -u); do
        conf=/proc/sys/net/ipv6/conf/$dev
        [ "$(cat "$conf/accept_ra")" = 1 ] && [ "$(cat "$conf/forwarding")" = 0 ] || continue
        if grep -Eqs "^[[:space:]]*iface[[:space:]]+${dev}[[:space:]]+inet6([[:space:]]|$)" /etc/network/interfaces /etc/network/interfaces.d/*; then
            ipv6_decline "$dev receives IPv6 router advertisements through ifupdown, which would ignore them once IPv6 forwarding is enabled. Set 'accept_ra 2' (ifupdown) or 'ipv6-accept-ra 2' (ifupdown-ng) in its inet6 stanza and apply it."
            return
        fi
        printf '2\n' | root tee "$conf/accept_ra" >/dev/null
        printf 'net/ipv6/conf/%s/accept_ra = 2\n' "$dev" | root tee -a /etc/sysctl.d/99-cdelivery-ipv6.conf >/dev/null
        say "Kept IPv6 router advertisements on $dev (accept_ra = 2, persisted in /etc/sysctl.d/99-cdelivery-ipv6.conf)."
    done
}

ipv6_gateway() {
    [ "$os" = Linux ] && [ "$ipv6" = yes ] || return 0
    awk '$4 == "00" && $6 !~ /^(docker|br-|veth)/ { found = 1 } END { exit !found }' /proc/net/if_inet6 2>/dev/null || return 0
    gateway=$(dk network inspect docker_gwbridge --format '{{.EnableIPv6}} {{len .Containers}}' 2>/dev/null) || gateway=missing
    case "$gateway" in
        true\ *) return 0 ;;
        missing|'false 0') ra_prepare ;;
        *) ipv6_decline "docker_gwbridge is IPv4-only and in use. Enabling IPv6 requires this server to leave its cluster first." ;;
    esac
    [ "$ipv6" = yes ] || return 0
    [ "$gateway" = missing ] || dk network rm docker_gwbridge >/dev/null
    error=$(dk network create --ipv6 \
        --opt com.docker.network.bridge.name=docker_gwbridge \
        --opt com.docker.network.bridge.enable_icc=false \
        --opt com.docker.network.bridge.enable_ip_masquerade=true \
        docker_gwbridge 2>&1 >/dev/null) || ipv6_decline "Cannot create the IPv6 gateway: $error"
}

ipv6_reminder() {
    [ "$ipv6" != skipped ] || say "Warning: the IPv6 gateway is not enabled; Pingress sees IPv6 clients as the gateway's IPv4 address."
}

join_cluster() {
    response=$(curl -sS --max-time 30 -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
        --data "{\"advertise_addr\":\"$addr\"}" -w ' %{http_code}' "$join_url/join/redeem") || die "Cannot reach $join_url."
    [ "${response##* }" = 200 ] || die "Invitation redemption failed: ${response% *}"
    result=$(printf '%s' "${response% *}" | as_docker curl -sS --unix-socket "$socket" -H 'Content-Type: application/json' \
        --data-binary @- -w ' %{http_code}' "http://localhost/v$api/swarm/join") || die "Cannot reach Docker at $socket."
    [ "${result##* }" = 200 ] || die "Cannot join the cluster: ${result% *}"
}

deploy() {
    [ "$state" = active ] || dk swarm init --advertise-addr "$addr" >/dev/null
    dk network inspect cdelivery >/dev/null 2>&1 ||
        dk network create --driver overlay --label dev.cdelivery.managed=true cdelivery >/dev/null
    dk service create --detach --name cdelivery \
        --label dev.cdelivery.managed=true --container-label dev.cdelivery.managed=true \
        --constraint node.role==manager --stop-grace-period 50s --update-monitor 60s --rollback-monitor 60s \
        --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock \
        --network cdelivery --publish published=9000,target=9000 "$image" >/dev/null
}

wait_ready() {
    say "Waiting for CDelivery to become ready (up to 3 minutes)."
    retry 60 3 curl -fs --noproxy '*' --max-time 3 -o /dev/null http://127.0.0.1:9000/setup && return
    dk service ps cdelivery --no-trunc >&2 || :
    die "CDelivery did not become ready within 3 minutes. Inspect the service tasks above."
}

show_dashboard() {
    host=$addr
    [ "$os" = Linux ] || host=localhost
    say "Open http://$host:9000 to set up or sign in to CDelivery."
    ipv6_reminder
}

main() {
    addr=
    join_url=
    token=
    ipv6=yes
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --advertise-addr|--join|--token)
                [ "$#" -ge 2 ] || die "$1 requires a value."
                case "$1" in
                    --advertise-addr) addr=$2 ;;
                    --join) join_url=${2%/} ;;
                    --token) token=$2 ;;
                esac
                shift ;;
            --no-ipv6) ipv6=no ;;
            --help) usage; return ;;
            *) die "Unknown argument: $1. Use --help." ;;
        esac
        shift
    done
    case "$addr" in *[!0-9.]*) die "--advertise-addr requires an IPv4 address." ;; esac
    case "$join_url" in ''|http://?*|https://?*) ;; *) die "--join requires an HTTP(S) URL." ;; esac
    case "$token" in ''|cdj_*) ;; *) die "--token requires a join invitation." ;; esac
    case "$token" in *[!A-Za-z0-9_=-]*) die "--token contains invalid characters." ;; esac
    case "${join_url:+join}${token:+token}" in ''|jointoken) ;; *) die "--join and --token must be used together." ;; esac

    os=$(uname -s)
    case "$os" in Linux|Darwin) ;; *) die "Only Linux and macOS are supported." ;; esac
    command -v curl >/dev/null 2>&1 || die "curl is required."
    docker_wait=1
    if ! command -v docker >/dev/null 2>&1; then
        [ "$os" = Linux ] || die "Docker is not installed. Install and start OrbStack (brew install --cask orbstack && orb start) or Docker Desktop, then rerun."
        install_docker
    fi
    connect_docker
    state=$(dk info --format '{{.Swarm.LocalNodeState}}')

    if [ -n "$join_url" ]; then
        [ "$state" = inactive ] || die "This server is already part of a cluster (state: $state)."
        choose_addr
        ipv6_gateway
        join_cluster
        say "This server joined the cluster."
        ipv6_reminder
        return
    fi

    case "$state" in
        active)
            [ "$(dk info --format '{{.Swarm.ControlAvailable}}')" = true ] || die "This server is a worker. Run the installer on a manager."
            current=$(dk info --format '{{.Swarm.NodeAddr}}')
            [ -z "$addr" ] || [ "$addr" = "$current" ] || die "This server already communicates through $current; --advertise-addr cannot change it."
            addr=$current
            if dk service inspect cdelivery >/dev/null 2>&1; then
                wait_ready
                say "CDelivery is already installed."
                show_dashboard
                return
            fi ;;
        inactive) choose_addr ;;
        *) die "The cluster state is '$state'. Resolve it, then rerun." ;;
    esac

    check_port
    release_image
    say "Pulling $image."
    dk pull "$image"
    ipv6_gateway
    deploy
    wait_ready
    say "CDelivery is ready."
    show_dashboard
}

main "$@"
