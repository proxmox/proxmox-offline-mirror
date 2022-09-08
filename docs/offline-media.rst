Offline Media
=============

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

Syncing a Medium
----------------

To sync the local mirrors to a medium, the following command can be used:

.. code-block:: console
  
  proxmox-offline-mirror medium sync --id pve-bullseye

This command will sync all mirrors linked with this medium to the medium's mountpoint, as well as
sync all offline keys for further processing by ``proxmox-apt-repo`` on the target system.

Using a Medium
--------------

After syncing a medium, unmount it and make it accessible on the (offline)
target system. You can now either manually point apt at the synced snapshots,
or run ``proxmox-apt-repo setup`` to generate a sources.list.d snippet referecing
selected mirrors and snapshots. Don't forget to remove the snippet again after
the upgrade is done.

To activate or update an offline subscription key, either use ``proxmox-apt-repo offline-key`` or
``proxmox-apt-repo setup``.
