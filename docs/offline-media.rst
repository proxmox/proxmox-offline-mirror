Offline Media
=============

A medium is a file system location on which mirrored repositories and subscription information can
be saved at to make it available to the offline Proxmox systems.  This can be an external portable
disk (for example a USB pen drive) or a local network share.

Setting Up a Medium
-------------------

Either run the ``setup`` wizard again, or use the ``config media add`` command.
For example, to define a new medium containing the
`proxmox-ve-bookworm-no-subscription` and `debian-bookworm` mirrors, run the
following command:

.. code-block:: console

  proxmox-offline-mirror config media add \
   --id pve-bookworm \
   --mirrors proxmox-ve-bookworm-no-subscription \
   --mirrors debian-bookworm \
   --sync true \
   --verify true \
   --mountpoint /path/where/medium/is/mounted

.. _sync_medium:

Syncing a Medium
----------------

To sync the local mirrors to a medium, the following command can be used:

.. code-block:: console

  proxmox-offline-mirror medium sync pve-bookworm

This command will sync all mirrors linked with this medium to the medium's mount point.
Additionally, it will sync all offline keys for further processing by
``proxmox-offline-mirror-helper`` on the target system.

Using a Medium
--------------

After syncing a medium you can make it accessible on the (offline) target system by either:

* unmounting it from the mirror host, plugging it into the target and mounting it there
* exposing it either directly (from the mirror host) or indirectly (from another host) via the local
  network (NFS, CIFS/SMB or HTTP)

After that you can upgrade the host as normally, either using the console or the web interface, as
described in the respective project's documentation.

Once an upgrade is done you should disable, or remove, the snippet to avoid apt errors from
automatic refresh. Note that you can disable and re-enable repositories simply via the web interface
of Proxmox VE, Proxmox Backup Server or Proxmox Mail Gateway.

Example: Local Mount Point
^^^^^^^^^^^^^^^^^^^^^^^^^^

Either option below assumes that you already mounted the medium on the target host.

Guided Setup
++++++++++++

For a guided setup run ``proxmox-offline-mirror-helper setup``. It will let you select the mirrors
and snapshots and can then generate a `sources.list.d` snippet. This snippet can be saved to the
``/etc/apt/sources.list.d`` directory. The default file name is ``offline-mirror.list``.

Manual Setup
++++++++++++

You can also just point `apt` directly at the synced snapshot folder(s) on the medium and then
create an new repository entry in a ``/etc/apt/sources.list.d`` file, for example:

.. code-block:: sources.list

  deb [check-valid-until=false] file:///mnt/mirror-path/<mirror-name>/<snapshot-timestamp> <codename> <suite>

Where ``<codename>`` is normally the Debian one, for example ``bullseye`` or ``bookworm`` and ``suite`` is
one or more of either the one of a Debian or Proxmox project, for example ``pve-enterprise``.
`pbstest` or `main`.

Now you should be able to upgrade like normally, and don't forget to disable the repository entry
again until next time, once your done.

Example: Local HTTP Server
^^^^^^^^^^^^^^^^^^^^^^^^^^

You can also configure an HTTP server to provide the snapshots in your internal network.
A minimal sample configuration for `nginx`:

  .. literalinclude:: examples/nginx-conf

The corresponding ``/etc/apt/sources.list.d`` file should contain

.. code-block:: sources.list

  deb [ check-valid-until=false ] http://proxmox-offline-mirror.domain.example/<mirror-name>/<snapshot-timestamp> <codename> <suite>

See `Manual Setup`_ above for examples abut what ``<codename>`` or ``<suite>`` can be.

Now you should be able to upgrade like normally, and don't forget to disable the repository entry
again until next time, once your done.

Activating an Subscription Key
------------------------------

To activate or update a subscription key offline, either use ``proxmox-offline-mirror-helper
offline-key`` directly or follow the respective step when doing the guided setup via the
``proxmox-offline-mirror-helper setup`` command.
