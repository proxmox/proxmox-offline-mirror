Offline Subscription Keys
=========================

When using ``proxmox-offline-mirror`` with a corresponding Proxmox Offline Mirror subscription key,
it is possible to update subscription information for air-gapped systems, or those without access
to the public internet.

First, add the `pom-<keyid>` mirror key using ``proxmox-offline-mirror key add-mirror-key <key>``.
This command will activate the subscription of the mirroring system.

.. note:: You can acquire a Promxox Offline Mirror Subscription key by contacting
   <sales@proxmox.com>. If the majority of your Proxmox VE, Proxmox Backup Server or
   Proxmox Mail Gateway hosts got standard or premium subscriptions you may be elligible for free
   offline mirroring subscription, in that case also write a mail to <sales@proxmox.com> for details.

Next, gather the server IDs of the systems that shall be set up for offline keys, and add them
together with the system's subscription key using ``proxmox-offline-mirror key add``. By default,
this command will fetch updated subscription information from Proxmox licensing servers.

You can refresh the subscription information for a single (``--key XX``) or all configured keys
using ``proxmox-offline-mirror key refresh``. The subscription information is transferred to a
medium (see below) and can then be activated on the offline system with either
``proxmox-apt-repo offline-key`` or ``proxmox-apt-repo setup``. This process must be repeated at least
once a year or before the nextduedate of the subscription key is reached, whichever comes first.

.. note:: Configuring an active product subscription key (*as well as* a Proxmox Offline Mirror
   subscription) is required for ``proxmox-offline-mirror`` to be able to access and mirror a
   product's enterprise repository.
