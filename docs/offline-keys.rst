Offline Subscription Keys
=========================

When using ``proxmox-offline-mirror`` with a corresponding Proxmox Offline Mirror subscription key,
it is possible to update subscription information for air-gapped systems, or those without access
to the public internet.

The offline mirror tool will take care of registering the subscription keys in the Proxmox shop,
which then responds with a signed data blob, if the key is valid. The signed response can then get
exported to a offline medium and used to set the subscription key in a Proxmox project, without any
need for an internet connection on the host itself.

Setup Offline Mirror Key
------------------------

First, add the `pom-<keyid>` mirror key using ``proxmox-offline-mirror key add-mirror-key <key>``.
This command will activate the subscription of the mirroring system.

.. note:: You can acquire a Promxox Offline Mirror Subscription key by contacting
   <sales@proxmox.com>. If the majority of your Proxmox VE, Proxmox Backup Server or
   Proxmox Mail Gateway hosts got standard or premium subscriptions you may be elligible for free
   offline mirroring subscription, in that case also write a mail to <sales@proxmox.com> for details.

Gather Server IDs
-----------------

Next, gather the server IDs of the systems that shall be set up for offline keys. That information
is visible in the subscription panel of each host, or using the CLI like:

- ``pvesubscription get`` for Proxmox VE

- ``proxmox-backup-manager subscription get`` for Proxmox Backup server

- ``pmgsubscription get`` for Proxmox Mail Gateway

Register & Refresh Keys
-----------------------

.. note:: Configuring an active product subscription key (*as well as* a Proxmox Offline Mirror
   subscription) is required for ``proxmox-offline-mirror`` to be able to access and mirror a
   product's enterprise repository.

Register the hosts with the systems's server IDs and subscription keys using
``proxmox-offline-mirror key add``, for example:

.. code-block:: console

  proxmox-offline-mirror key add pve2p-12345... ABCDEF0123...

By default, this command will fetch updated subscription information from Proxmox licensing servers.

You can refresh the subscription information for a single (``--key XX``) or all configured keys
using ``proxmox-offline-mirror key refresh``.

Deploy Keys
-----------

The subscription information is transferred to a medium (see :ref:`sync_medium`) and can then be
activated on the offline system with either ``proxmox-apt-repo offline-key`` or ``proxmox-apt-repo
setup``. This process must be repeated at least once a year or before the nextduedate of the
subscription key is reached, whichever comes first.
