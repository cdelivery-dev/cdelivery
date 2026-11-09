# Prepare ZFS for CDelivery

Run on each target Ubuntu/Debian server with systemd and rootful Docker. The running Linux kernel must support ZFS. On macOS or Windows, use a suitable Linux VM or server. For unattended execution, run as root. Communicate in the user's language.

1. Inspect the target and storage:

   ```sh
   hostname
   lsblk --bytes --output PATH,TYPE,SIZE,FSTYPE,FSAVAIL,MOUNTPOINTS
   ```

   The installer reuses or imports an existing `cdelivery` pool. Otherwise, use an unused partition of at least 64 MiB. Reuse the user's chosen partition; if none was chosen, show the devices and sizes and ask. `FSAVAIL` is space inside a filesystem, not unpartitioned space.

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
   zpool status -x cdelivery
   zfs list -o name,mountpoint,available cdelivery/volumes cdelivery/guardian
   ```

   Expect a healthy pool and `mountpoint=none` on both parent datasets. Report the server and available space. The user can then enable Volumes with ZFS in CDelivery. Existing ordinary volumes are not migrated.

The script exits nonzero on failure. Resolve the reported error and rerun; do not wipe a partition or destroy a pool to bypass it. New servers use the generated join command; `--zfs-partition` can be added to that command for unattended storage preparation.
