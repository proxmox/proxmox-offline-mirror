Introduction
============

What is Proxmox Offline Mirror?
-------------------------------

With Proxmox Offline Mirror (POM) you can manage a local apt mirror for all Proxmox and Debian
projects package updates. You can also export this mirror to a external medium to update systems
that cannot connect to the repositories via HTTP, for example due to being completely air-gapped.
Finally you can also manage subscription for such restricted hosts.

This tool consists of two binaries:

``proxmox-offline-mirror``
  The mirror tool to create and manage mirrors and media containing repositories

``proxmox-apt-repo``
  The helper to use media and setup subscription key on offline Proxmox VE,
  Proxmox Mail Gateway or Proxmox Backup Server systems

Terminology
-----------

There are three basic entity types available for configuration:

*keys*
  Subscription keys are either for the mirroring system itself, or for the offline systems.

  They are configured with ``proxmox-offline-mirror key ...``

*mirrors*
  A mirror consists of the metadata of an upstream repository and a local path where **snapshots**
  of the upstream repository are stored.

  - configured with ``proxmox-offline-mirror config mirror ...``

  - used with ``proxmox-offline-mirror mirror ...``

*media*
  A medium consisting of local mirrors and a path where the mirrors are synced to

  - configured with ``proxmox-offline-mirror config medium ...``

  - used with ``proxmox-offline-mirror medium ...``


Technical Overview
------------------

Behind the scenes, one or more `pools` consisting of

- a pool directory containing checksum files (e.g., `sha256/3dc7bc5f82cdcc4ea0f69dd30d5f6bb19e0ccc36f4a79c865eed0e7a370cd5e4`)
- a base directory containing directories and hardlinks to the checksum files inside the pool
  directory

are used for space-efficient storing of repository contents ("snapshots").

Adding a file consists of first adding the checksum file(s), then linking them under one or more
paths. a garbage collect operation will iterate over all files in the base directory and remove
those which are not (or no longer) a hardlink to any checksum files, and remove any checksum files
which have no hardlinks outside of the pool checksum file directories.

A default config path of ``/etc/proxmox-offline-mirror.cfg`` is used, but is overridable on a per
command basis (for example, to allow operation as non-root user) using either the ``--config`` CLI
option or the ``PROXMOX_OFFLINE_MIRROR_CONFIG`` environment variable.


.. _get_help:

Getting Help
------------

.. _get_help_enterprise_support:

Enterprise Support
^^^^^^^^^^^^^^^^^^

Users with a `Proxmox Offline Mirror` subscription have access to the `Proxmox Customer Portal
<https://my.proxmox.com>`_ for offline mirroring/key handling related issues, provided the
corresponding offline system has a valid subscription level higher than `Community`. The customer
portal provides support with guaranteed response times from the Proxmox developers.

For more information or for volume discounts, please contact office@proxmox.com.

Community Support Forum
^^^^^^^^^^^^^^^^^^^^^^^

We always encourage our users to discuss and share their knowledge using the
`Proxmox Community Forum`_. The forum is moderated by the Proxmox support team.
The large user base is spread out all over the world. Needless to say that such
a large forum is a great place to get information.

Mailing Lists
^^^^^^^^^^^^^

Proxmox Offline Mirror is fully open-source and contributions are welcome! The Proxmox VE
development mailing list acts as the primary communication channel for offline mirror developers:

:Mailing list for developers: `PVE Development List`_

Bug Tracker
^^^^^^^^^^^

Proxmox runs a public bug tracker at `<https://bugzilla.proxmox.com>`_. If an
issue appears, file your report there. An issue can be a bug, as well as a
request for a new feature or enhancement. The bug tracker helps to keep track
of the issue and will send a notification once it has been solved.

License
-------

|pom-copyright|

This software is written by Proxmox Server Solutions GmbH <support@proxmox.com>

Proxmox Offline Mirror is free and open source software: you can use it,
redistribute it, and/or modify it under the terms of the GNU Affero General
Public License as published by the Free Software Foundation, either version 3
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but
``WITHOUT ANY WARRANTY``; without even the implied warranty of
``MERCHANTABILITY`` or ``FITNESS FOR A PARTICULAR PURPOSE``.  See the GNU
Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see AGPL3_.
