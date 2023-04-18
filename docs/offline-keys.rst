Offline Subscription Keys
=========================

The ``proxmox-offline-mirror`` tool can be used to manage subscription keys for air-gapped systems
or systems that cannot access the public internet. To use this functionality, you need a
subscription key for Proxmox Offline Mirror itself.

The offline mirror tool will take care of registering the subscription keys in the Proxmox shop.  If
the key is valid, it will get a signed data blob in response. The signed response can then be
exported to an offline medium to transfer and set the subscription key in a Proxmox solution,
without any need for an internet connection on the host itself.


Minimum Versions for Offline Activation
---------------------------------------

Offline activation of subscription keys requires support from the respective Proxmox solution. The
following table shows in which version this support has been added.

=====================  =====================  ======================================
Solution               Package                Minimum Version
=====================  =====================  ======================================
Proxmox VE             pve-manager            7.2-11
Proxmox Backup Server  proxmox-backup-server  2.2.6-1
Proxmox Mail Gateway   pmg-api                7.1-7
=====================  =====================  ======================================

.. _setup_offline_key:

Setup Offline Mirror Key
------------------------

First, add the `pom-<keyid>` mirror key using ``proxmox-offline-mirror key add-mirror-key <key>``.
This command will activate the subscription of the mirroring system.

.. note:: To purchase a subscription key for Proxmox Offline Mirror, please contact
   <sales@proxmox.com>. If you already have a Standard or Premium subscription for the majority of
   your Proxmox VE, Proxmox Backup Server or Proxmox Mail Gateway hosts, you may be eligible for a
   free Offline Mirror subscription. In that case, please email <sales@proxmox.com> to
   get more details.

Gather Server IDs
-----------------

Next, gather the server IDs of the systems that will be set up for offline keys. You can see the
server ID in the subscription panel of each host, or by using the CLI with the following commands:

- ``pvesubscription get`` for Proxmox VE

- ``proxmox-backup-manager subscription get`` for Proxmox Backup server

- ``pmgsubscription get`` for Proxmox Mail Gateway

Register & Refresh Keys
-----------------------

.. note:: To be able to access and mirror a product's enterprise repository,
   ``proxmox-offline-mirror`` requires that both, an active product subscription key and a Proxmox
   Offline Mirror subscription is configured.

Register the hosts with their subscription keys and server IDs using
``proxmox-offline-mirror setup`` or ``proxmox-offline-mirror key add``, for
example:

.. code-block:: console

  proxmox-offline-mirror key add pve2p-12345... ABCDEF0123...

By default, this command will fetch the updated subscription information from the Proxmox
subscription servers.

You can refresh the subscription information for a single (``--key XX``) or all configured keys
using ``proxmox-offline-mirror key refresh``.

Deploy Keys
-----------

The subscription information is transferred to a medium (see :ref:`sync_medium`) and can then be
activated on the offline system with either ``proxmox-offline-mirror-helper offline-key`` or
``proxmox-offline-mirror-helper setup``. This process must be repeated at least once a year or
before the next due date of the subscription key is reached, whichever comes first.
