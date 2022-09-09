Offline Media
=============

A medium is a file system location on which mirrored repositories and subscription information can
be saved at to make it available to the offline Proxmox systems.  This can be an external portable
disk (for example a USB pen drive) or a local network share.

Setting Up a Medium
-------------------

Either run the ``setup`` wizard again, or use the ``config medium add`` command.
For example, to define a new medium containing the
`proxmox-ve-bullseye-no-subscription` and `debian-bullseye` mirrors, run the
following command:

.. code-block:: console

  proxmox-offline-mirror config medium add \
   --id pve-bullseye \
   --mirrors proxmox-ve-bullseye-no-subscription \
   --mirrors debian-bullseye \
   --sync true \
   --verify true \
   --mountpoint /path/where/medium/is/mounted

.. _sync_medium:

Syncing a Medium
----------------

To sync the local mirrors to a medium, the following command can be used:

.. code-block:: console

  proxmox-offline-mirror medium sync --id pve-bullseye

This command will sync all mirrors linked with this medium to the medium's mount point.
Additionally, it will sync all offline keys for further processing by ``proxmox-offline-mirror-helper`` on the
target system.

Using a Medium
--------------

After syncing a medium, unmount it and make it accessible on the (offline) target system.  Either
point `apt` directly at the synced snapshots on the medium or run ``proxmox-offline-mirror-helper setup``.  The
setup will let you select the mirrors and snapshots and can generate a `sources.list.d` snippet.
This snippet can be saved to the ``/etc/apt/sources.list.d`` directory. The default file name is
``offline-mirror.list``.  Don't forget to remove the snippet after the upgrade is done.

To activate or update an offline subscription key, either use ``proxmox-offline-mirror-helper offline-key`` or
``proxmox-offline-mirror-helper setup``.
