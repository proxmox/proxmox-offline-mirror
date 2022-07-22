Introduction
============

What is Proxmox Offline Mirror?
-------------------------------

This tool consists of two binaries, `proxmox-offline-mirror` (mirror tool to create
and manage mirrors and media containing repositories) and `proxmox-apt-repo`
(helper to use media on offline systems).

There are two basic entity types available for configuration:
- mirrors, consisting of upstream repository metadata and a local path where snapshots are stored
-- configured with `proxmox-offline-mirror config mirror ...`
-- used with `proxmox-offline-mirror mirror ...`
- media, consisting of local mirrors and a path where mirrors are synced to
-- configured with `proxmox-offline-mirror config medium ...`
-- used with `proxmox-offline-mirror medium ...`

and one internal one, a `pool` consisting of
- a pool directory containing checksum files (e.g., `sha256/3dc7bc5f82cdcc4ea0f69dd30d5f6bb19e0ccc36f4a79c865eed0e7a370cd5e4`)
- a base directory containing directories and hardlinks to checksum files inside the pool directory

Adding a file consists of first adding the checksum file(s), then linking them
under one or more paths. a garbage collect operation will iterate over all
files in the base directory and remove those which are not (or no longer) a
hardlink to any checksum files, and remove any checksum files which have no
hardlinks outside of the pool checksum file directories.

A default config path of `/etc/proxmox-offline-mirror.cfg` is used, but is
overridable on a per command basis (for example, to allow operation as non-root
user).

Offline subscription keys
=========================

When using `proxmox-offline-mirror` with a corresponding Proxmox Offline Mirror subscription key,
it is possible to update subscription information for air-gapped systems or those without access
to the public internet.
 
First, add the mirror key using `proxmox-offline-mirror key add-mirror-key`. This command will
activate the subscription of the mirroring system.
 
Next, gather the server IDs of the systems that shall be set up for offline keys, and add them
together with the system's subscription key using `proxmox-offline-mirror key add`. By default,
this command will fetch updated subscription information from Proxmox licensing servers.

You can refresh the subscription information for a single (`--key XX`) or all configured keys
using `proxmox-offline-mirror key refresh`. The subscription information is transferred to a
medium (see below) and can then be activated on the offline system with either
`proxmox-apt-repo offline-key` or `proxmox-apt-repo setup`. This process must be repeated at least
once a year or before the nextduedate of the subscription key is reached, whichever comes first.

.. note:: Configuring an active product subscription key (*as well as* a Proxmox Offline Mirror
   subscription) is required for `proxmox-offline-mirror` to be able to access and mirror a
   product's enterprise repository.

Offline repository mirrors
==========================

Setting up a mirror
-------------------

First either run the `setup` wizard (`proxmox-offline-mirror setup`), or the
`config mirror add` command. For example, to add a mirror entry for the Debian
Bullseye security repository, the following command can be used:

 proxmox-offline-mirror config mirror add \
  --id debian-bullseye-security \
  --architectures amd64 \
  --architectures all \
  --repository 'deb http://deb.debian.org/debian-security bullseye-security main contrib non-free' \
  --key-path /etc/apt/trusted.gpg.d/debian-archive-bullseye-security-automatic.gpg \
  --sync true \
  --verify true \
  --base-dir /path/to/mirror/dir/debian-bullseye-security \
  --pool-dir /path/to/mirror/dir/debian-bullseye-security/.pool

Syncing a mirror
----------------

To create the first (and subsequent) snapshots, the following command can be used:

 proxmox-offline-mirror mirror snapshot create --id debian-bullseye-security

Offline media
=============

Setting up a medium
-------------------

Either run the `setup` wizard again, or use the `config medium add` command.
For example, to define a new medium containing the
`proxmox-ve-bullseye-no-subscription` and `debian-bullseye` mirrors, run the
following command:

 proxmox-offline-mirror config medium add \
  --id pve-bullseye \
  --mirrors proxmox-ve-bullseye-no-subscription \
  --mirrors debian-bullseye \
  --sync true \
  --verify true \
  --mountpoint /path/where/medium/is/mounted

Syncing a medium
----------------

To sync the local mirrors to a medium, the following command can be used:

 proxmox-offline-mirror medium sync --id pve-bullseye

Using a medium
--------------

After syncing a medium, unmount it and make it accessible on the (offline)
target system. You can now either manually point apt at the synced snapshots,
or run `proxmox-apt-repo setup` to generate a sources.list.d snippet referecing
selected mirrors and snapshots. Don't forget to remove the snippet again after
the upgrade is done.


.. _get_help:

Getting Help
------------

.. _get_help_enterprise_support:

Enterprise Support
~~~~~~~~~~~~~~~~~~

Users with a `Proxmox Offline Mirror Basic, Standard or Premium Subscription Plan
<https://www.proxmox.com/en/proxmox-offline-mirror/pricing>`_ have access to the
`Proxmox Customer Portal <https://my.proxmox.com>`_. The customer portal
provides support with guaranteed response times from the Proxmox developers.
For more information or for volume discounts, please contact office@proxmox.com.

Community Support Forum
~~~~~~~~~~~~~~~~~~~~~~~

We always encourage our users to discuss and share their knowledge using the
`Proxmox Community Forum`_. The forum is moderated by the Proxmox support team.
The large user base is spread out all over the world. Needless to say that such
a large forum is a great place to get information.

Mailing Lists
~~~~~~~~~~~~~

Proxmox Offline Mirror is fully open-source and contributions are welcome! Here
is the primary communication channel for developers:

:Mailing list for developers: `PVE Development List`_

Bug Tracker
~~~~~~~~~~~

Proxmox runs a public bug tracker at `<https://bugzilla.proxmox.com>`_. If an
issue appears, file your report there. An issue can be a bug, as well as a
request for a new feature or enhancement. The bug tracker helps to keep track
of the issue and will send a notification once it has been solved.

License
-------

|pom-copyright|

This software is written by Proxmox Server Solutions GmbH <support@proxmox.com>

Proxmox Backup Server is free and open source software: you can use it,
redistribute it, and/or modify it under the terms of the GNU Affero General
Public License as published by the Free Software Foundation, either version 3
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but
``WITHOUT ANY WARRANTY``; without even the implied warranty of
``MERCHANTABILITY`` or ``FITNESS FOR A PARTICULAR PURPOSE``.  See the GNU
Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see AGPL3_.