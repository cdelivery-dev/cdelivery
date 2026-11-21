# Prepare ZFS for CDelivery

Run on each target Ubuntu/Debian server with systemd 245 or later and rootful Docker 28 or later. The running Linux kernel must support ZFS. On macOS or Windows, use a suitable Linux VM or server. For unattended execution, run as root. Communicate in the user's language.

ZFS is optional. Without it, ordinary volumes and other metrics continue to work. Agents use the host's public system D-Bus to run its installed `/usr/sbin/zfs` and `/usr/sbin/zpool` through short-lived systemd services. No helper daemon or command container is installed. The managed agent services run as host UID 0 with AppArmor disabled and read-only host runtime-directory mounts; these mounts grant access to the host's privileged system bus. Keep the agents and their images trusted. Later agent upgrades normally only require new images.

Insight automatically samples imported pools and filesystem/volume datasets every five minutes alongside filesystem capacity. Unavailable D-Bus or ZFS skips that collection and retries next round. Pool physical allocation, dataset usage and filesystem capacity are separate measurements and must not be added together. Management operations report live command failures. Existing preview data is preserved on repeated preparation; clean up that preview explicitly before preparing it again.

1. Inspect the target and storage:

   ```sh
   hostname
   lsblk --bytes --output PATH,TYPE,SIZE,FSTYPE,FSAVAIL,MOUNTPOINTS
   df -B1 --output=source,fstype,size,used,avail,target /
   ```

   The installer reuses or imports an existing `cdelivery` pool, even when no unused storage is available. Otherwise, use an unused partition of at least 64 MiB. Reuse the user's chosen partition; if none was chosen, show the devices and sizes and ask.

   Distinguish these capacities:

   - A `disk` row's `SIZE` is the whole disk capacity.
   - A `part` row's `SIZE` is the capacity already assigned to that partition.
   - `FSAVAIL` and `df` available space are inside a filesystem. Deleting files increases this space without changing partition boundaries.
   - Unpartitioned space is outside existing partitions. A new ZFS partition needs a contiguous extent of at least 64 MiB after alignment; filesystem free space cannot be used directly.

   To inspect unpartitioned space on a disk with a GPT/MBR partition table, replace `/dev/CHOSEN_DISK` with its actual disk path and run this read-only command as root (requires `parted`):

   ```sh
   parted --script /dev/CHOSEN_DISK unit MiB print free
   ```

   A root partition can occupy almost the entire disk while `/` still has ample filesystem free space. If no eligible storage is found, choose one of these next steps:

   - Add a data disk, then rerun `--zfs` to select storage.
   - Expand an existing disk and leave the added capacity unpartitioned. Do not extend the root or another existing partition/filesystem into that space; check whether the server image automatically grows the root partition.
   - To reclaim space from an ext4 root partition, arrange a backup and downtime, boot a rescue environment, and shrink the unmounted filesystem before shrinking its partition. Handle this separately, then rerun the installer. The installer does not shrink filesystems, unmount the root filesystem, or reboot the server.

   On simple layouts with no evidence of existing ZFS storage, the installer checks for candidates before installing ZFS or compiling its kernel module. It may first install `parted`. Existing ZFS tools, a loaded module, a pool cache, unfamiliar signatures, or complex device layouts defer this check until ZFS can inspect pools. A failed detection command is reported with its original error, not treated as proof that no space exists.

2. Download the installer, then run it with the confirmed partition:

   ```sh
   (
     set -eu
     installer=$(mktemp)
     trap 'rm -f "$installer"' 0
     curl -fsSL https://cdelivery.dev/install.sh -o "$installer"
     sh "$installer" --zfs-partition /dev/CHOSEN_PARTITION
   )
   ```

   Replace `/dev/CHOSEN_PARTITION` with the actual path. `--zfs-partition` implies `--zfs` and confirms the choice without a prompt. It accepts only an unused partition; it does not erase signatures, shrink filesystems, or repartition a disk. An existing pool takes precedence over this option.

   With a terminal, use `--zfs` instead to select an unused partition, empty disk, or unpartitioned space and choose a new partition's size. Without a terminal, prepare a partition only after its device and size have been agreed, then use `--zfs-partition`. If a pool already exists, `--zfs` also works without a terminal.

3. Verify before reporting success:

   ```sh
   systemctl is-active dbus
   zpool status -x cdelivery
   zfs list -o name,mountpoint,available cdelivery/volumes cdelivery/guardian
   ```

   Expect a healthy pool and `mountpoint=none` on both parent datasets. Report the server and available space. The user can then enable Volumes with ZFS in CDelivery. Existing ordinary volumes are not migrated.

The script exits nonzero on failure. Resolve the reported error and rerun; do not wipe a partition or destroy a pool to bypass it. New servers use the generated join command; `--zfs-partition` can be added to that command for unattended storage preparation.
