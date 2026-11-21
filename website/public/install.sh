#!/bin/sh
set -eu

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
Usage: install.sh [--image-prefix REGISTRY[/NAMESPACE]] [--advertise-addr IPv4] [--no-ipv6]
       install.sh --zfs [--zfs-partition PATH]
       install.sh --join URL --token INVITATION [--zfs] [--advertise-addr IPv4] [--no-ipv6]

Install CDelivery, or join this server to an existing CDelivery instance.

  --image-prefix PREFIX  Install from the release catalog for this image prefix.
                         Defaults to ghcr.io/cdelivery-dev.
  --advertise-addr IPv4  Use this server communication address without a menu.
                         Your other servers must be able to reach it.
                         127.0.0.1 is for a single server only.
  --join URL             Join the CDelivery instance at this HTTP(S) origin.
  --token INVITATION     Use its join invitation.
  --zfs                 Prepare partition-backed ZFS storage; without --join,
                         only prepare storage on this server.
  --zfs-partition PATH   Use this unused partition without prompting (implies --zfs).
                         An existing cdelivery pool is reused instead.
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

choose() (
    { exec 3<>/dev/tty; } 2>/dev/null || exit 1
    title=$1 selected=$2 options=$3
    count=$(printf '%s\n' "$options" | awk 'END { print NR }')
    printf '\n%s\n' "$title" >&3
    trap 'exit 130' INT
    trap 'exit 129' HUP
    trap 'exit 143' TERM

    keys=no highlight= reset=
    if [ -n "${TERM:-}" ] && [ "$TERM" != dumb ] && command -v dd >/dev/null 2>&1 &&
        settings=$(stty -g <&3 2>/dev/null); then
        trap 'stty "$settings" <&3; printf "%s\n" "$reset" >&3' 0
        if stty -icanon -echo min 1 time 0 <&3 2>/dev/null; then
            keys=yes
        else
            stty "$settings" <&3 || exit 1
        fi
    fi

    if [ "$keys" = no ]; then
        printf '%s\n' "$options" | awk '{ printf "  %d) %s\n", NR, $0 }' >&3
        while :; do
            printf '1-%s [Enter = %s]: ' "$count" "$selected" >&3
            IFS= read -r answer <&3 || exit 1
            answer=${answer:-$selected}
            case "$answer" in *[!0-9]*) continue ;; esac
            if [ "$answer" -ge 1 ] && [ "$answer" -le "$count" ]; then
                printf '%s\n' "$answer" | awk '{ print $1 + 0 }'
                exit 0
            fi 2>/dev/null
        done
    fi

    if [ -z "${NO_COLOR:-}" ]; then
        highlight=$(printf '\033[1;36m')
        reset=$(printf '\033[0m')
    fi
    printf '%s\n' "${4:-Use Up/Down to choose, Enter to confirm, Ctrl+C to cancel.}" >&3
    escape=$(printf '\033')
    carriage=$(printf '\r')
    eof=$(printf '\004')
    while :; do
        printf '%s\n' "$options" | awk -v selected="$selected" -v highlight="$highlight" -v reset="$reset" '{
            printf "\r\033[2K%s%s %d) %s%s\n", (NR == selected ? highlight : ""), (NR == selected ? ">" : " "), NR, $0, reset
        }' >&3
        sequence=
        while :; do
            key=$(dd bs=1 count=1 <&3 2>/dev/null && printf '.') || exit 1
            key=${key%.}
            case "$key" in
                ''|"$eof") exit 1 ;;
                '
'|"$carriage") printf '%s\n' "$selected"; exit 0 ;;
                "$escape") sequence=$escape; continue ;;
            esac
            sequence=$sequence$key
            case "$sequence" in
                "$escape["|"${escape}O") continue ;;
                "$escape[A"|"${escape}OA") [ "$selected" -le 1 ] || selected=$((selected - 1)); break ;;
                "$escape[B"|"${escape}OB") [ "$selected" -ge "$count" ] || selected=$((selected + 1)); break ;;
            esac
            sequence=
        done
        printf '\033[%sA' "$count" >&3
    done
)

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
            root apt-get update
            policy=$(LC_ALL=C apt-cache policy docker.io)
            candidate=$(printf '%s\n' "$policy" | awk '$1 == "Candidate:" { print $2; exit }')
            if [ -n "$candidate" ] && [ "$candidate" != '(none)' ] && dpkg --compare-versions "${candidate#*:}" ge 28.0; then
                say "Installing docker.io $candidate from the configured APT sources."
                set -- "docker.io=$candidate"
                [ "$ID" != debian ] || set -- "$@" docker-cli
            else
                say "No compatible docker.io candidate. Using Docker's official APT repository."
                curl -fsSI "$repo/dists/$VERSION_CODENAME/Release" >/dev/null || die "$unavailable"
                root install -d -m 0755 /etc/apt/keyrings
                root curl -fsSL "$repo/gpg" -o /etc/apt/keyrings/docker.asc
                root chmod a+r /etc/apt/keyrings/docker.asc
                printf 'Types: deb\nURIs: %s\nSuites: %s\nComponents: stable\nArchitectures: %s\nSigned-By: /etc/apt/keyrings/docker.asc\n' \
                    "$repo" "$VERSION_CODENAME" "$(dpkg --print-architecture)" | root tee /etc/apt/sources.list.d/docker.sources >/dev/null
                root apt-get update
                set -- docker-ce docker-ce-cli containerd.io
            fi
            root env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-remove "$@"
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
    answer=$(choose "Select a stable server communication address that your other servers can reach:" 1 "$list") ||
        die "Address selection stopped. Use an interactive terminal or rerun with --advertise-addr IPv4."
    addr=$(printf '%s\n' "$list" | awk -v choice="$answer" 'NR == choice { print $1 }')
}

check_port() {
    if [ "$os" = Linux ]; then
        cat /proc/net/tcp /proc/net/tcp6 2>/dev/null | awk '$4 == "0A" && $2 ~ /:2328$/ { exit 1 }'
    else
        ! lsof -nP -iTCP:9000 -sTCP:LISTEN >/dev/null
    fi || die "TCP port 9000 is already in use."
}

release_image() {
    catalog=https://releases.cdelivery.dev/v1/$image_prefix/latest.json
    data=$(curl -fsSL --max-time 30 --max-filesize 131072 "$catalog") || die "Cannot download $catalog."
    image=$(printf '%s' "$data" | tr -d '\r\n' | awk '{
        if (!match($0, /"cdelivery"[[:space:]]*:[[:space:]]*\{/)) exit
        s = substr($0, RSTART + RLENGTH)
        if (match(s, /"image"[[:space:]]*:[[:space:]]*"[^"]*"/)) {
            value = substr(s, RSTART, RLENGTH)
            sub(/^"image"[[:space:]]*:[[:space:]]*"/, "", value)
            print substr(value, 1, length(value) - 1)
        }
    }')
    tag=${image##*:}
    [ "$image" = "$image_prefix/cdelivery:$tag" ] && printf '%s\n' "$tag" | grep -Eqx 'v?[0-9]+\.[0-9]+\.[0-9]+' ||
        die "Cannot read a valid $image_prefix/cdelivery image from $catalog."
}

ipv6_decline() {
    say "$1"
    say "Without an IPv6 gateway, Pingress sees IPv6 clients as the gateway's IPv4 address."
    answer=$(choose "Choose how to proceed:" 2 'Continue with IPv4 only.
Exit, resolve it, then rerun.') || answer=
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

zfs_unused() (
    device=$1
    children=$(lsblk -nrpo PATH "$device") || { say "Cannot list devices below $device."; return 2; }
    [ "$children" = "$device" ] || return 1
    mounts=$(lsblk -dnro MOUNTPOINTS "$device") || { say "Cannot inspect mounts on $device."; return 2; }
    signatures=$(root wipefs --no-act --noheadings --output TYPE "$device") || { say "Cannot inspect signatures on $device."; return 2; }
    holders=$(ls -A "/sys/class/block/${device##*/}/holders") || { say "Cannot inspect holders of $device."; return 2; }
    [ -z "$mounts$signatures$holders" ] || return 1
    swaps=$(cat /proc/swaps) || { say "Cannot inspect active swap devices."; return 2; }
    swap=$(printf '%s\n' "$swaps" | awk -v device="$device" 'NR > 1 && $1 == device { print $1 }') || { say "Cannot inspect active swap devices."; return 2; }
    [ -z "$swap" ] || return 1
    partition_type=$(lsblk -dnro PARTTYPE "$device") || { say "Cannot inspect the partition type of $device."; return 2; }
    case "$partition_type" in
        ''|0x83|0fc63daf-8483-4772-8e79-3d69d8477de4|6a898cc3-1dd2-11b2-99a6-080020736631) return 0 ;;
        *) return 1 ;;
    esac
)

zfs_candidates() (
    devices=$(lsblk -nrbo PATH,TYPE,SIZE) || { say "Cannot list block devices for ZFS."; return 2; }
    while read -r path kind bytes; do
        case "$kind" in disk|part) ;; *) continue ;; esac
        [ "$bytes" -ge 67108864 ] || continue
        unused=0
        zfs_unused "$path" || unused=$?
        [ "$unused" -ne 2 ] || return 2
        case "$kind" in
            part)
                if [ "$unused" -eq 0 ]; then
                    printf 'partition|%s|0|%s\n' "$path" "$((bytes / 1048576))"
                fi ;;
            disk)
                if [ "$bytes" -ge 69206016 ] && [ "$unused" -eq 0 ]; then
                    printf 'disk|%s|1|%s\n' "$path" "$((bytes / 1048576 - 2))"
                else
                    table=$(lsblk -dnro PTTYPE "$path") || { say "Cannot inspect the partition table on $path."; return 2; }
                    case "$table" in
                        gpt|dos) ;;
                        *) say "$path: unpartitioned space was not assessed (no supported GPT/MBR partition table)."; continue ;;
                    esac
                    layout=$(root parted --machine --script "$path" unit MiB print free) || { say "Cannot inspect unpartitioned space on $path."; return 2; }
                    printf '%s\n' "$layout" | awk -F: -v disk="$path" '
                        $NF == "free;" {
                            extents++; total += $4
                            start = int($2 + 0.999999); end = int($3)
                            if (start < 1) start = 1
                            if (end - start >= 64) {
                                found = 1
                                printf "space|%s|%d|%d\n", disk, start, end - start
                            }
                        }
                        END {
                            if (!extents) printf "%s: no unpartitioned space reported.\n", disk > "/dev/stderr"
                            else if (!found) printf "%s: %.3f MiB unpartitioned in total, but no aligned extent of at least 64 MiB.\n", disk, total > "/dev/stderr"
                        }' || return 2
                fi ;;
        esac
    done <<EOF
$devices
EOF
)

zfs_no_space() {
    lsblk --bytes --output PATH,TYPE,SIZE,FSTYPE,FSAVAIL,MOUNTPOINTS >&2 || die "Cannot inspect the storage layout."
    say "SIZE is disk or partition capacity in bytes; FSAVAIL is free space inside a filesystem."
    say "Root filesystem usage (bytes):"
    df -B1 --output=source,fstype,size,used,avail,target / >&2 || die "Cannot inspect root filesystem usage."
    say "Free space on / remains inside its filesystem and partition; it is not unpartitioned space for ZFS. Deleting files does not change partition boundaries."
    say "Next steps:"
    say "  1. Add a data disk, then rerun --zfs to select storage."
    say "  2. Expand an existing disk and leave the added capacity unpartitioned; do not extend its existing partition/filesystem into that space."
    say "  3. To reclaim space from an ext4 root partition, arrange a backup and downtime, boot a rescue environment, and shrink the unmounted filesystem before shrinking its partition. This must be handled separately."
    die "No eligible unused partition, empty disk, or unpartitioned extent of at least 64 MiB was found. See https://cdelivery.dev/zfs.md."
}

prepare_zfs() {
    [ "$os" = Linux ] && [ -d /run/systemd/system ] || die "ZFS requires Linux with systemd."
    . /etc/os-release
    case "$ID" in ubuntu|debian) ;; *) die "Automatic ZFS preparation supports Ubuntu and Debian." ;; esac
    security=$(dk info --format '{{json .SecurityOptions}}') || die "Cannot inspect Docker security options."
    case "$security" in *rootless*) die "ZFS requires rootful Docker." ;; esac
    preflight=no
    if ! command -v zpool >/dev/null 2>&1 && ! command -v zfs >/dev/null 2>&1 &&
        [ ! -d /sys/module/zfs ] && [ ! -e /etc/zfs/zpool.cache ] &&
        command -v lsblk >/dev/null 2>&1 && command -v wipefs >/dev/null 2>&1; then
        devices=$(lsblk -nrpo PATH,TYPE) || die "Cannot list block devices for the ZFS storage preflight."
        preflight=yes
        while read -r path kind; do
            case "$kind" in disk|part) ;; *) preflight=no; continue ;; esac
            signatures=$(root wipefs --no-act --noheadings --output TYPE "$path") || die "Cannot inspect signatures on $path during the ZFS storage preflight."
            for signature in $signatures; do
                case "$signature" in gpt|PMBR|dos|ext4|vfat) ;; *) preflight=no ;; esac
            done
        done <<EOF
$devices
EOF
    fi
    if ! command -v parted >/dev/null 2>&1; then
        root apt-get update
        root env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-remove parted
    fi
    if [ "$preflight" = yes ]; then
        say "Checking storage before installing ZFS."
        candidates=$(zfs_candidates) || die "ZFS storage detection failed; available space could not be determined."
        [ -n "$candidates" ] || zfs_no_space
    else
        say "Storage preflight deferred: ZFS may already exist or the layout cannot be ruled out without ZFS tools. Existing pools will be checked before selecting new storage."
    fi
    if ! command -v zfs >/dev/null 2>&1 || ! command -v zpool >/dev/null 2>&1; then
        if [ "$ID" = debian ]; then
            printf 'Types: deb\nURIs: https://deb.debian.org/debian\nSuites: %s\nComponents: contrib\nSigned-By: /usr/share/keyrings/debian-archive-keyring.gpg\n' "$VERSION_CODENAME" |
                root tee /etc/apt/sources.list.d/cdelivery-zfs.sources >/dev/null
        fi
        root apt-get update
        root env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-remove zfsutils-linux
    fi
    if ! root modprobe zfs; then
        root env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-remove "linux-headers-$(uname -r)" zfs-dkms
        root modprobe zfs || die "Cannot load ZFS for the running kernel. Resolve the module error and rerun."
    fi
    pools=$(root zpool list -H -o name) || die "Cannot list ZFS pools."
    pool=$(printf '%s\n' "$pools" | awk '$0 == "cdelivery" { print }') || die "Cannot read the ZFS pool list."
    if [ -z "$pool" ]; then
        imports=$(root zpool import) || die "Cannot scan for importable ZFS pools."
        pool=$(printf '%s\n' "$imports" | awk '$1 == "pool:" && $2 == "cdelivery" { print $2 }') || die "Cannot read the importable ZFS pool list."
        if [ -n "$pool" ]; then
            root zpool import -N cdelivery || die "Cannot import the existing cdelivery pool."
        else
            candidates=$(zfs_candidates) || die "ZFS storage detection failed; available space could not be determined."
            [ -n "$candidates" ] || zfs_no_space
            lsblk --bytes --output PATH,TYPE,SIZE,FSTYPE,FSAVAIL,MOUNTPOINTS >&2 || die "Cannot inspect the storage layout."
            say "Choose an unused partition or unpartitioned space. FSAVAIL is space inside a filesystem, not available for partitioning."
            if [ -n "$zfs_partition" ]; then
                zfs_partition=$(readlink -f -- "$zfs_partition") || die "Cannot resolve --zfs-partition."
                row=$(printf '%s\n' "$candidates" | awk -F'|' -v path="$zfs_partition" '$1 == "partition" && $2 == path')
                [ -n "$row" ] || die "--zfs-partition must select an unused partition of at least 64 MiB. See https://cdelivery.dev/zfs.md."
            else
                menu=$(printf '%s\n' "$candidates" | awk -F'|' '{ printf "%s: %s, %s MiB available, start %s MiB\n", $2, $1, $4, $3 }')
                selected=$(choose "Select storage for ZFS:" 1 "$menu") || {
                    say "$menu"
                    die "Selection stopped. Use a terminal or --zfs-partition PATH for an unused partition. See https://cdelivery.dev/zfs.md."
                }
                row=$(printf '%s\n' "$candidates" | awk -v n="$selected" 'NR == n')
            fi
            action=$(printf '%s\n' "$row" | cut -d'|' -f1)
            disk=$(printf '%s\n' "$row" | cut -d'|' -f2)
            start=$(printf '%s\n' "$row" | cut -d'|' -f3)
            size=$(printf '%s\n' "$row" | cut -d'|' -f4)
            if [ "$action" != partition ]; then
                { exec 3<>/dev/tty; } 2>/dev/null || die "A terminal is required to choose the new partition size."
                printf 'Partition size in MiB [%s]: ' "$size" >&3
                IFS= read -r requested <&3 || die "Storage selection stopped."
                requested=${requested:-$size}
                case "$requested" in *[!0-9]*|'') die "Enter an integer size in MiB." ;; esac
                [ "$requested" -ge 64 ] && [ "$requested" -le "$size" ] || die "Size must be at least 64 MiB and within the selected free extent."
                size=$requested
            fi
            if [ -z "$zfs_partition" ]; then
                { exec 3<>/dev/tty; } 2>/dev/null || die "A terminal is required to confirm storage creation."
                printf 'Create ZFS storage on %s (%s MiB, %s)? Type YES: ' "$disk" "$size" "$action" >&3
                IFS= read -r confirmation <&3 || die "Storage preparation stopped."
                exec 3>&-
                [ "$confirmation" = YES ] || die "Storage preparation cancelled."
            fi
            case "$action" in
                partition) partition=$disk ;;
                disk|space)
                    if [ "$action" = disk ]; then
                        unused=0
                        zfs_unused "$disk" || unused=$?
                        [ "$unused" -ne 2 ] || die "Cannot verify that $disk is unused."
                        [ "$unused" -eq 0 ] || die "Disk is no longer unused."
                        root parted --script "$disk" mklabel gpt
                        table=gpt
                    else
                        table=$(lsblk -dnro PTTYPE "$disk") || die "Cannot inspect the partition table on $disk."
                    fi
                    case "$table" in gpt) label=cdelivery ;; dos) label=primary ;; *) die "Unsupported partition table." ;; esac
                    root parted --script --align optimal "$disk" mkpart "$label" "${start}MiB" "$((start + size))MiB"
                    root partprobe "$disk" || die "Partition created, but the kernel could not reload it. Reboot, then select the new partition."
                    root udevadm settle
                    layout=$(root parted --machine --script "$disk" unit MiB print) || die "Cannot read the partition table after creating a partition on $disk. Inspect the disk before retrying."
                    number=$(printf '%s\n' "$layout" | awk -F: -v start="$start" '$1 ~ /^[0-9]+$/ && $2 + 0 == start { print $1 }')
                    devices=$(lsblk -nrpo PATH,TYPE "$disk") || die "Cannot list partitions on $disk after creation. Inspect the disk before retrying."
                    partition=
                    while read -r path kind; do
                        [ "$kind" = part ] || continue
                        current_number=$(cat "/sys/class/block/${path##*/}/partition") || die "Cannot read the partition number of $path. Inspect the disk before retrying."
                        if [ "$current_number" = "$number" ]; then partition=$path; fi
                    done <<EOF
$devices
EOF
                    [ -n "$partition" ] || die "New partition could not be identified. Inspect the disk before retrying." ;;
            esac
            unused=0
            zfs_unused "$partition" || unused=$?
            [ "$unused" -ne 2 ] || die "Cannot verify that $partition is unused."
            [ "$unused" -eq 0 ] || die "Partition is no longer empty or unused."
            root zpool create -m none cdelivery "$partition"
        fi
    fi
    root zpool status -LP cdelivery >&2
    datasets=$(root zfs list -H -o name -r cdelivery) || die "Cannot list datasets in the cdelivery pool."
    for dataset in cdelivery/volumes cdelivery/guardian; do
        existing=$(printf '%s\n' "$datasets" | awk -v name="$dataset" '$0 == name { print }') || die "Cannot read the dataset list."
        if [ -n "$existing" ]; then
            mountpoint=$(root zfs get -H -o value mountpoint "$dataset") || die "Cannot inspect the mountpoint of $dataset."
            [ "$mountpoint" = none ] || die "$dataset must have mountpoint=none; existing properties were not changed."
        else
            root zfs create -u -o mountpoint=none "$dataset"
        fi
    done
    root install -d -m 0755 /etc/zfs /etc/modules-load.d /etc/systemd/system/docker.service.d
    root zpool set cachefile=/etc/zfs/zpool.cache cdelivery
    printf 'zfs\n' | root tee /etc/modules-load.d/cdelivery-zfs.conf >/dev/null
    printf '[Unit]\nWants=zfs-import-cache.service\nAfter=zfs-import-cache.service\n' |
        root tee /etc/systemd/system/docker.service.d/cdelivery-zfs.conf >/dev/null
    root systemctl daemon-reload
    root systemctl enable zfs-import-cache.service
    say "ZFS ready: cdelivery/volumes and cdelivery/guardian."
}

main() {
    addr=
    image_prefix=ghcr.io/cdelivery-dev
    join_url=
    token=
    ipv6=yes
    zfs_enabled=no
    zfs_partition=
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --advertise-addr|--image-prefix|--join|--token|--zfs-partition)
                [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 requires a value."
                case "$1" in
                    --advertise-addr) addr=$2 ;;
                    --image-prefix) image_prefix=$2 ;;
                    --join) join_url=${2%/} ;;
                    --token) token=$2 ;;
                    --zfs-partition) zfs_enabled=yes; zfs_partition=$2 ;;
                esac
                shift ;;
            --no-ipv6) ipv6=no ;;
            --zfs) zfs_enabled=yes ;;
            --help) usage; return ;;
            *) die "Unknown argument: $1. Use --help." ;;
        esac
        shift
    done
    case "${image_prefix%%/*}" in localhost|*.*|*:*) ;; *) die "--image-prefix requires an explicit registry host." ;; esac
    case "$image_prefix" in *[!a-z0-9.:/_-]*) die "Invalid --image-prefix." ;; esac
    printf '%s\n' "$image_prefix" | grep -Eqx '[a-z0-9]+(-+[a-z0-9]+)*(\.[a-z0-9]+(-+[a-z0-9]+)*)*(:[0-9]+)?(/[a-z0-9]+(([._]|__|-+)[a-z0-9]+)*)*' ||
        die "--image-prefix requires a registry and optional namespace."
    case "$addr" in *[!0-9.]*) die "--advertise-addr requires an IPv4 address." ;; esac
    case "$zfs_partition" in ''|/dev/?*) ;; *) die "--zfs-partition requires an absolute /dev/ partition path." ;; esac
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
    if [ "$zfs_enabled" = yes ]; then
        prepare_zfs
        [ -n "$join_url" ] || return 0
    fi
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
    dk pull "$image" || die "Cannot pull $image. Check the registry connection or rerun and choose another source."
    ipv6_gateway
    deploy
    wait_ready
    say "CDelivery is ready."
    show_dashboard
}

case "${1:-}" in
    --choose) shift; choose "$@" ;;
    *) main "$@" ;;
esac
