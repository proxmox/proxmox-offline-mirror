use anyhow::{bail, format_err, Error};

use proxmox_http::client::sync::Client;
use proxmox_http::{HttpClient, HttpOptions, ProxyConfig};
use proxmox_subscription::SubscriptionStatus;
use proxmox_subscription::{
    sign::{SignRequest, SignedResponse},
    SubscriptionInfo,
};

use crate::{config::SubscriptionKey, types::ProductType};

// TODO: Update with final, public URL
const PRODUCT_URL: &str = "-";
// TODO add version?
const USER_AGENT: &str = "proxmox-offline-mirror";

fn client() -> Result<Client, Error> {
    let options = HttpOptions {
        user_agent: Some(USER_AGENT.to_string()),
        proxy_config: ProxyConfig::from_proxy_env()?,
        ..Default::default()
    };
    Ok(Client::new(options))
}

pub fn extract_mirror_key(keys: &[SubscriptionKey]) -> Result<SubscriptionKey, Error> {
    keys.iter()
        .find(|k| {
            if k.product() != ProductType::Pom {
                return false;
            }
            if let Ok(Some(info)) = k.info() {
                info.status == SubscriptionStatus::Active
            } else {
                false
            }
        })
        .ok_or_else(|| format_err!("No active mirror subscription key configured!"))
        .cloned()
}

/// Refresh `offline_keys` using `mirror_key`.
///
/// This consists of two phases:
/// 1. refresh all the offline keys (so that the info downloaded in step 3 is current)
/// 2. get updated signed blobs for all offline keys (for transfer to offline systems)
pub fn refresh_offline_keys(
    mirror_key: SubscriptionKey,
    mut offline_keys: Vec<SubscriptionKey>,
    public_key: openssl::pkey::PKey<openssl::pkey::Public>,
) -> Result<Vec<SubscriptionInfo>, Error> {
    let mut errors = false;

    offline_keys.retain(|k| k.product() != ProductType::Pom);
    if offline_keys.is_empty() {
        return Ok(vec![]);
    }

    for key in &offline_keys {
        if let Err(err) = proxmox_subscription::check::check_subscription(
            key.key.clone(),
            key.server_id.clone(),
            PRODUCT_URL.to_string(),
            client()?,
        ) {
            errors = true;
            eprintln!("Failed to refresh subscription key {} - {}", key.key, err);
        }
    }
    if errors {
        bail!("Refresh error - see above.");
    }
    let request = SignRequest {
        mirror_key: mirror_key.into(),
        blobs: offline_keys.into_iter().map(|k| k.into()).collect(),
    };
    let res = client()?.post(
        "https://shop.proxmox.com/proxmox-subscription/sign",
        Some(serde_json::to_vec(&request)?.as_slice()),
        Some("text/json"),
        None,
    )?;
    if res.status().is_success() {
        let body: Vec<u8> = res.into_body();
        let res: SignedResponse = serde_json::from_slice(&body)?;
        res.verify(&public_key)
    } else {
        bail!("Refresh failed - {}", res.status());
    }
}

/// Refresh a mirror key.
///
/// Should be called before calling `extract_mirror_key()` or
/// `refresh_offline_keys()` to ensure mirror key is (still) valid.
pub fn refresh_mirror_key(mirror_key: SubscriptionKey) -> Result<SubscriptionInfo, Error> {
    proxmox_subscription::check::check_subscription(
        mirror_key.key,
        mirror_key.server_id,
        PRODUCT_URL.to_string(),
        client()?,
    )
}
