Offline Repository Mirrors
==========================

Offline repository mirrors are pointing to APT repositories, for example from Proxmox VE, Proxmox
Backup Server or Debian. After the initial setup, you can mirror all the available packages locally.
They are organized by creating point-in-time snapshots of the repositories. Those snapshots can then
be exported to a configured medium.

Setting Up a Mirror
-------------------

First, either run the ``setup`` wizard (``proxmox-offline-mirror setup``), or the
``config mirror add`` command.

.. tip:: The quickest way to set up all relevant repositories for a Proxmox solution is to use the
   ``setup`` wizard. Choose the product when adding a mirror and confirm the question regarding
   auto-adding the Debian base repos.

For example, to manually add a mirror entry for the Debian Bullseye security repository, the
following command can be used:

.. code-block:: console

  proxmox-offline-mirror config mirror add \
   --id debian-bullseye-security \
   --architectures amd64 \
   --architectures all \
   --repository 'deb http://deb.debian.org/debian-security bullseye-security main contrib non-free' \
   --key-path /etc/apt/trusted.gpg.d/debian-archive-bullseye-security-automatic.gpg \
   --sync true \
   --verify true \
   --base-dir /path/to/mirror/base-dir

.. note:: The `base-dir` directory can be shared by mirrors for repositories that have common
   contents to avoid storing files more than once. For example, having a single base directory
   for all mirrors referencing Proxmox repositories is recommended.

.. note:: The `all` architecture is meant for architecture independent packages, not for all
   possible architectures. It is usually always sensible to add it in addition to the host-specific
   architecture.

Syncing a Mirror
----------------

To create the first (and subsequent) snapshots, the following command can be used:

.. code-block:: console

  proxmox-offline-mirror mirror snapshot create debian-bullseye-security

.. note:: Depending on the parameters used and the size of the original repository, creating a
  snapshot can take both time and require significant disk space. This is especially true for the
  initial snapshot, as subsequent ones will re-use unchanged package files and indices.

Space Management
----------------

After removing a snapshot with ``proxmox-offline-mirror mirror snapshot remove``, a
``proxmox-offline-mirror mirror gc`` invocation is needed to trigger the garbage collection to
actually remove any contents from the underlying hard link pool that are no longer needed.
