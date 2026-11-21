#!/bin/sh
set -eu

probe() (
    registry=${1%%/*} repository=${1#*/}/cdelivery auth=$2
    digest=sha256:7cc70dfd88cf2327bb827123c9156f7cb95de95b2f98f3a7a931296aacec5786
    export LC_ALL=C
    set -- -qfsSL --proto '=https' --proto-redir '=https'
    auth_time=$(curl "$@" --max-time 8 --max-filesize 65536 -o "$work/$registry.json" -w '%{time_total}' \
        "$auth?service=$registry&scope=repository:$repository:pull") || exit 1
    token=$(awk -F '"' '{ for (i = 2; i < NF; i += 2) if ($i == "token" || $i == "access_token") { print $(i + 2); exit } }' "$work/$registry.json")
    case "$token" in ''|*[![:graph:]]*) echo '获取匿名访问令牌失败' >&2; exit 1 ;; esac
    remaining=$(awk -v elapsed="$auth_time" 'BEGIN { if (elapsed >= 8) exit 1; printf "%.6f", 8 - elapsed }') ||
        { echo '测速超时' >&2; exit 1; }
    transfer=$(printf 'Authorization: Bearer %s\n' "$token" | curl "$@" --header @- --max-time "$remaining" \
        --range 0-524287 --max-filesize 524288 -o /dev/null -w '%{http_code} %{size_download} %{time_total}' \
        "https://$registry/v2/$repository/blobs/$digest") || exit 1
    printf '%s\n' "$transfer" | awk -v auth="$auth_time" '
        $1 == 206 && $2 == 524288 { printf "%.2f 秒\n", auth + $3; ok = 1 }
        END { if (!ok) { print "未收到完整的 512 KiB 测速样本" > "/dev/stderr"; exit 1 } }'
)

run() (
    official=ghcr.io/cdelivery-dev
    mirror=m.daocloud.io/$official
    mode=choose
    for arg do
        case "$arg" in
            --probe-only) [ "$#" -eq 1 ] || { echo '--probe-only 需单独使用' >&2; exit 1; }; mode=probe ;;
            --help) echo 'install-cn.sh --probe-only  仅测速，随后可用 --image-prefix 指定来源。'; mode=direct ;;
            --image-prefix|--join|--zfs|--zfs-partition) mode=direct ;;
        esac
    done
    work=$(mktemp -d "${TMPDIR:-/tmp}/cdelivery-cn.XXXXXX")
    trap 'rm -rf "$work"' 0
    trap 'exit 130' INT
    trap 'exit 143' HUP TERM
    if [ "$mode" != probe ]; then
        curl -qfsSL --proto '=https' --proto-redir '=https' --connect-timeout 15 --max-time 120 \
            https://cdelivery.dev/install.sh -o "$work/install.sh"
    fi
    while [ "$mode" != direct ]; do
        echo '正在比较加速安装和官方直连（每个源读取 512 KiB，限时 8 秒）…' >&2
        probe "$mirror" https://m.daocloud.io/auth/token >"$work/mirror" 2>&1 &
        mirror_pid=$!
        probe "$official" https://ghcr.io/token >"$work/official" 2>&1 &
        official_pid=$!
        selected=3
        if wait "$official_pid"; then selected=2; fi
        if wait "$mirror_pid"; then selected=1; fi
        mirror_result=$(paste -s -d ' ' "$work/mirror")
        official_result=$(paste -s -d ' ' "$work/official")
        case "$selected" in
            1) mirror_result="推荐，$mirror_result" ;;
            2) official_result="推荐，$official_result" ;;
        esac
        options="加速安装（DaoCloud）  $mirror_result
官方直连（GHCR）      $official_result"
        if [ "$mode" = probe ]; then
            printf '%s\n' "$options"
            [ "$selected" != 3 ] || return 1
            return 0
        fi
        if [ "$selected" = 3 ]; then options="$options
重新测速"; fi
        choice=$(sh "$work/install.sh" --choose '请选择 CDelivery 镜像来源' "$selected" "$options" \
            '使用 ↑↓ 选择，按回车确认，按 Ctrl+C 取消。') || {
            printf '%s\n' "$options" >&2
            echo '尚未选择镜像源。请在终端中选择，或先运行 --probe-only，再用 --image-prefix 指定来源。' >&2
            exit 1
        }
        case "$choice" in
            1) set -- --image-prefix "$mirror" "$@"; break ;;
            2) set -- --image-prefix "$official" "$@"; break ;;
        esac
    done
    sh "$work/install.sh" "$@"
)

run "$@"
