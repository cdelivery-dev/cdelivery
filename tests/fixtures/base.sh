#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
printf 'APT::Keep-Downloaded-Packages "true";\nBinary::apt::APT::Keep-Downloaded-Packages "true";\n' > /etc/apt/apt.conf.d/99cdelivery-tests
dpkg --configure -a
apt-get update
apt-get install --yes --no-install-recommends ca-certificates curl
install -m 0755 -d /etc/apt/keyrings
curl --fail --silent --show-error --retry 3 https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
printf 'deb [arch=%s signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable\n' "$(dpkg --print-architecture)" > /etc/apt/sources.list.d/docker.list
apt-get update
apt-get install --yes --no-install-recommends docker-ce docker-ce-cli containerd.io
/usr/bin/docker version
{ cat /etc/os-release; uname -m; dpkg-query -W docker-ce docker-ce-cli containerd.io; } > /root/cdelivery-test-base.versions
systemctl disable --now docker.service docker.socket containerd.service
rm -rf /var/lib/docker /var/lib/containerd
rm -f /var/lib/dbus/machine-id /var/lib/systemd/random-seed
truncate -s 0 /etc/machine-id
touch /root/cdelivery-test-base.ready
