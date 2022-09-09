Installation
============

Proxmox Offline Mirror is released as standard Debian package and is available in the Proxmox VE,
Proxmox Backup Server and Proxmox Mail Gateway package repositories.

System Requirements
-------------------

* CPU: 64bit (*x86-64* or *AMD64*), ideally 2+ Cores

* Debian based system (physical host, VM or container)

* Sufficient storage space for the local mirrors.
  For a basic Debian and Proxmox VE we recommend at least 150 GiB

* A file system supporting hard links for both, the local host and the external medium.  Note that
  most Linux derived file systems support hard links, but Windows derived ones (for example, \*FAT)
  do *not* support hard links.

.. _apt_install_pom:

Installation via APT
--------------------

If the host you want to install the ``proxmox-offline-mirror`` tools on, already has a package
repository from a Proxmox solution configured, you can simply install the offline mirror tool with
``apt``:

.. code-block:: console

     # apt update
     # apt install proxmox-offline-mirror

If you do not have any Proxmox VE, Proxmox Backup Server or Proxmox Mail Gateway repositories set
up, see :ref:`package_repos_secure_apt` and :ref:`package_repositories_client_only_apt` for how to
do so before using the commands above.

Debian Package Repositories
^^^^^^^^^^^^^^^^^^^^^^^^^^^

All Debian based systems use APT as a package management tool. The lists of repositories are
defined in ``/etc/apt/sources.list`` and the ``.list`` files found in the ``/etc/apt/sources.d/``
directory. Updates can be installed directly with the ``apt`` command line tool, or via the GUI.

APT ``sources.list`` files list one package repository per line, with the most preferred source
listed first. Empty lines are ignored, and a ``#`` character anywhere on a line marks the remainder
of that line as a comment. The information available from the configured sources is acquired by
``apt update``.

.. _package_repos_secure_apt:

SecureApt
^^^^^^^^^

The `Release` files in the repositories are signed with GnuPG. APT is using these signatures to
verify that all packages are from a trusted source.

.. tip:: If you install Proxmox Offline Mirror on an existing Proxmox VE, Proxmox Backup Server or
   Proxmox Mail Gateway, the verification key will already be present.

If you install Proxmox Offline Mirror on top of Debian Bullseye, download and install the key with
the following commands:

.. code-block:: console

 # wget https://enterprise.proxmox.com/debian/proxmox-release-bullseye.gpg \
   -O /etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg

Verify the SHA512 checksum afterwards with the expected output below:

.. code-block:: console

 # sha512sum /etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg
 7fb03ec8a1675723d2853b84aa4fdb49a46a3bb72b9951361488bfd19b29aab0a789a4f8c7406e71a69aabbc727c936d3549731c4659ffa1a08f44db8fdcebfa  /etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg

or the md5sum, with the expected output below:

.. code-block:: console

 # md5sum /etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg
 bcc35c7173e0845c0d6ad6470b70f50e /etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg

.. _package_repositories_client_only_apt:

Set up the Repository on non Proxmox based systems
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

If you do not have an internet facing installation of a Proxmox solution, or want to set up a
dedicated system for Proxmox Offline Mirror, you need to configure the repository first.

This should work on any Linux distribution using `apt` as package manager, such as Debian, Ubuntu or
derivatives thereof.

To configure the repository, you first need to :ref:`set up the Proxmox release key
<package_repos_secure_apt>`. After that, add the repository URL to the APT sources lists.

We recommend re-using the ``pbs-client`` repository for installing the Proxmox Offline Mirror on non
Proxmox systems.

.. hint:: While you could also use a Proxmox VE, Proxmox Backup Server or Proxmox Mail Gateway
   repository, those ship some updated packages from Debian native packages, which would get pulled
   in, even if not required for the offline mirroring.

Repository for Debian 11 (Bullseye) based releases
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Here are the actual steps for a generic Debian 11 (Bullseye) based system.

First edit the file ``/etc/apt/sources.list.d/pbs-client.list`` and add the following snippet:

.. code-block:: sources.list
  :caption: File: ``/etc/apt/sources.list.d/pbs-client.list``

  deb http://download.proxmox.com/debian/pbs-client bullseye main

Now you should be able to install the ``proxmox-offline-mirror`` package, see
:ref:`apt_install_pom`.
