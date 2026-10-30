#!/bin/sh
set -eu
registry=$1
mkdir -p /etc/docker
printf '{"insecure-registries":["%s"]}\n' "$registry" > /etc/docker/daemon.json
systemctl enable --now containerd.service docker.service
if [ -n "${2-}" ]; then
    cat > /etc/systemd/system/cdelivery-test-api.service <<EOF
[Unit]
After=docker.service

[Service]
ExecStart=/usr/bin/systemd-socket-activate --listen=[::]:$2 /usr/lib/systemd/systemd-socket-proxyd 127.0.0.1:9000
Restart=on-failure

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable --now cdelivery-test-api.service
fi
/usr/bin/docker version
