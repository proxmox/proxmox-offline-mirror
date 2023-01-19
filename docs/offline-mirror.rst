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

Reducing Mirror Scope
---------------------

There are different mechanisms for reducing a mirror's scope (and correspondingly, the amount of
traffic and disk space required to keep it synced):

- architecture filters
- components (as part of the `repository` specification)
- package name and section filters

By default, only packages for the architectures `all` (see note above) and `amd64` are mirrored.

Optionally, it's possible to setup filters for downloaded binary or source packages via the
`--skip-packages` and `--skip-sections` options. The package filters support globbing, for example
`linux-image-*` will skip all packages with a name starting with `linux-image-`. The section
filters match the full value, or the value prefixed with the package's component (for example,
`games` will match both the section `games`, as well as `non-free/games` in a packages index of the
`non-free` component).

Some examples for packages and section filters:

- `--skip-packages 'linux-image-*'` - filter Debian linux kernel image packages
- `--skip-sections 'games'` - filter sections containing game packages
- `--skip-sections 'debug'` - filter sections containing debug information

Please refer to https://packages.debian.org/bullseye/ for a list of Debian archive sections and
their contents.

Space Management
----------------

After removing a snapshot with ``proxmox-offline-mirror mirror snapshot remove``, a
``proxmox-offline-mirror mirror gc`` invocation is needed to trigger the garbage collection to
actually remove any contents from the underlying hard link pool that are no longer needed.

.. _env_vars :

Environment Variables
---------------------


``ALL_PROXY``
  When set, the client uses the specified HTTP proxy for all connections to the
  backup server. Currently only HTTP proxies are supported. Valid proxy
  configurations have the following format:
  `[http://][user:password@]<host>[:port]`. Default `port` is 1080, if not
  otherwise specified.

.. Note:: The proxy server must allow ``HTTP CONNECT`` for all ports that are used
   to connect to mirrors (e.g. port 80 for HTTP mirrors). For Squid,
   the appropriate configuration parameter is ``http_access allow CONNECT <acl>``
   (http://www.squid-cache.org/Doc/config/http_access/). By default, Squid only
   allows ``HTTP CONNECT`` for port 443.


.. Note:: Passwords must be valid UTF-8 and may not contain newlines. For your
   convenience, Proxmox Backup Server only uses the first line as password, so
   you can add arbitrary comments after the first newline.
