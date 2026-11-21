#!/bin/sh
set -euf

die() {
    printf 'Error: %s\nResolve the error and rerun uninstall.sh.\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: uninstall.sh [--help]

Preview resources on the selected local Docker daemon, then confirm removal.
If no managed resources remain, exit without prompting.
Enter y to confirm removal; any other input cancels.
Confirmation requires an interactive terminal and cannot be skipped.
Run on a manager with Docker CLI and jq available.
All data volumes and volume descriptions are preserved.
Wait up to 120 seconds for managed networks to disappear from the local network list.
On timeout, report remaining networks and exit with an error.
Containers exit asynchronously; exit on offline nodes cannot be confirmed.
Do not install or recreate resources while uninstall is running.
If deletion fails, resolve the error and rerun the script.
EOF
}

owned() {
    docker "$1" ls --filter label=dev.cdelivery.managed=true --format '{{.Name}}'
}

node_labels() {
    value=$(docker node inspect --format '{{json .Spec.Labels}}' -- "$1") || exit 1
    printf '%s\n' "$value" | jq -r '
        (. // {}) | keys[] | select(startswith("dev.cdelivery.labels."))
        | if test("[\u0000-\u001f\u007f]") then error("Control character in node label key") else . end'
}

stop_main() {
    printf 'Stopping cdelivery.\n'
    docker service scale --detach=true cdelivery=0
    deadline=$(( $(date +%s) + 120 ))
    while :; do
        tasks=$(docker service ps --no-trunc -q -- cdelivery)
        waiting=no
        for task in $tasks; do
            task_state=$(docker inspect --type task --format '{{.Status.State}} {{.NodeID}}' -- "$task")
            state=${task_state%% *}
            task_node=${task_state#* }
            case "$state" in complete|shutdown|failed|rejected) continue ;; esac
            [ -n "$task_node" ] || continue
            node_state=$(docker node inspect --format '{{.Status.State}}' -- "$task_node")
            if [ "$node_state" != ready ]; then
                printf 'Warning: container exit is unconfirmed on node %s (%s).\n' "$task_node" "$node_state" >&2
                continue
            fi
            waiting=yes
        done
        [ "$waiting" = yes ] || break
        [ "$(date +%s)" -lt "$deadline" ] || die 'Main service tasks have not exited within 120 seconds.'
        sleep 1
    done
    replicas=$(docker service inspect --format '{{.Spec.Mode.Replicated.Replicas}}' -- cdelivery)
    [ "$replicas" = 0 ] || die 'The main service replica count changed; its service was retained.'
    docker service rm -- cdelivery
}

case "$#:$*" in
    0:) ;;
    1:--help) usage; exit 0 ;;
    *) printf 'Use uninstall.sh [--help].\n' >&2; exit 2 ;;
esac
for dependency in docker jq; do
    command -v "$dependency" >/dev/null 2>&1 || die "Missing required command: $dependency."
done
if [ -n "${DOCKER_CONTEXT:-}" ]; then
    endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}' -- "$DOCKER_CONTEXT")
elif [ -n "${DOCKER_HOST:-}" ]; then
    endpoint=$DOCKER_HOST
else
    endpoint=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
fi
case "$endpoint" in
    unix:///*) ;;
    *) die 'Select a local Docker Unix socket.' ;;
esac
unset DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH DOCKER_API_VERSION
export DOCKER_HOST="$endpoint"
printf 'Using %s.\n' "$endpoint"
manager=$(docker info --format '{{.Swarm.ControlAvailable}}')
[ "$manager" = true ] || die 'Run this script on a manager.'

found=no
for kind in service config secret network; do
    names=$(owned "$kind")
    [ -z "$names" ] || found=yes
    for name in $names; do printf 'Selected %s: %s\n' "$kind" "$name"; done
done
nodes=$(docker node ls -q)
for node in $nodes; do
    labels=$(node_labels "$node")
    if [ -n "$labels" ]; then
        found=yes
        printf 'Selected placement labels on node %s:\n%s\n' "$node" "$labels"
    fi
done
if [ "$found" = no ]; then
    printf 'No CDelivery resources to remove. All volumes and volume descriptions are preserved.\n'
    exit 0
fi
printf 'All volumes and volume descriptions are preserved. Application definitions, accounts and credentials are removed.\n'
{ command exec 3<>/dev/tty; } 2>/dev/null || die 'Confirmation requires an interactive terminal.'
trap 'exit 130' INT
printf 'Uninstall the listed resources? [y/N]: ' >&3
IFS= read -r answer <&3 || die 'Confirmation stopped.'
exec 3>&-
case "$answer" in
    y|Y) ;;
    *) printf 'Uninstall cancelled. No resources were removed.\n'; exit 0 ;;
esac

services=$(owned service)
for service in $services; do
    if [ "$service" = cdelivery ]; then stop_main; fi
done
services=$(owned service)
for service in $services; do
    case "$service" in cdelivery-insight-agent|cdelivery-insight) continue ;; esac
    docker service rm -- "$service"
done
for component in cdelivery-insight-agent cdelivery-insight; do
    for service in $services; do
        if [ "$service" = "$component" ]; then docker service rm -- "$service"; fi
    done
done
for kind in config secret network; do
    names=$(owned "$kind")
    for name in $names; do docker "$kind" rm -- "$name"; done
done
nodes=$(docker node ls -q)
for node in $nodes; do
    labels=$(node_labels "$node")
    [ -n "$labels" ] || continue
    (
        set -- node update
        IFS='
'
        for label in $labels; do set -- "$@" --label-rm "$label"; done
        docker "$@" -- "$node"
    )
done
names=$(owned network)
if [ -n "$names" ]; then
    printf 'Waiting for network cleanup...\n'
    deadline=$(( $(date +%s) + 120 ))
    while [ -n "$names" ]; do
        [ "$(date +%s)" -lt "$deadline" ] || die "Networks still present after 120 seconds: $names"
        sleep 1
        names=$(owned network)
    done
fi
printf 'Management resources removed. All volumes and volume descriptions were preserved.\n'
printf 'Container cleanup is asynchronous; exit on offline nodes is not confirmed.\n'
