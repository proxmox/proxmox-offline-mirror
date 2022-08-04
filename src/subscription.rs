use anyhow::{bail, format_err, Error};

use proxmox_http::client::sync::Client;
use proxmox_http::{HttpClient, HttpOptions};
use proxmox_subscription::{
    sign::{SignRequest, SignedResponse},
    SubscriptionInfo,
};

use crate::{config::SubscriptionKey, types::ProductType};

const PRODUCT_URL: &str = "ADD URL FOR PROXMOX-APT-MIRROR";
// TODO add version?
const USER_AGENT: &str = "proxmox-offline-mirror";

fn client() -> Client {
    let options = HttpOptions {
        user_agent: Some(USER_AGENT.to_string()),
        ..Default::default()
    };
    Client::new(options)
}

pub fn extract_mirror_key(keys: &[SubscriptionKey]) -> Result<SubscriptionKey, Error> {
    keys.iter()
        .find(|k| k.product() == ProductType::Pom)
        .ok_or_else(|| format_err!("No mirror subscription key configured!"))
        .cloned()
}

/// Refresh `offline_keys` using `mirror_key`.
///
/// This consists of two phases:
/// 1.
pub async fn refresh(
    mirror_key: SubscriptionKey,
    mut offline_keys: Vec<SubscriptionKey>,
    public_key: openssl::pkey::PKey<openssl::pkey::Public>,
) -> Result<Vec<SubscriptionInfo>, Error> {
    let mut errors = false;

    let mirror_info = proxmox_subscription::check::check_subscription(
        mirror_key.key.clone(),
        mirror_key.server_id.clone(),
        PRODUCT_URL.to_string(),
        client(),
    )?;
    offline_keys.retain(|k| k.product() != ProductType::Pom);
    if offline_keys.is_empty() {
        return Ok(vec![mirror_info]);
    }

    for key in &offline_keys {
        if let Err(err) = proxmox_subscription::check::check_subscription(
            key.key.clone(),
            key.server_id.clone(),
            PRODUCT_URL.to_string(),
            client(),
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
    let res = client().post(
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
