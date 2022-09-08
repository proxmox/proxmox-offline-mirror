Configuration Files
===================

The Proxmox Offline Mirror configuration file is stored at ``/etc/proxmox-offline-mirror.cfg`` by default. Its location can be overriden for any given command using the `--config` parameter.

Note that you can use the ``--config <file>`` switch on most commands or the ``PROXMOX_OFFLINE_MIRROR_CONFIG`` environment variable to override the default config location.


``proxmox-offline-mirror.cfg``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~


Options
^^^^^^^

.. include:: config/mirror/config.rst
