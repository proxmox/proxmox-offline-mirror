use anyhow::{bail, Error};

use sequoia_openpgp::{
    parse::{
        stream::{
            DetachedVerifierBuilder, MessageLayer, MessageStructure, VerificationHelper,
            VerifierBuilder,
        },
        Parse,
    },
    policy::StandardPolicy,
    Cert, KeyHandle,
};
use std::io;

struct Helper<'a> {
    cert: &'a Cert,
}

impl<'a> VerificationHelper for Helper<'a> {
    fn get_certs(&mut self, _ids: &[KeyHandle]) -> sequoia_openpgp::Result<Vec<Cert>> {
        // Return public keys for signature verification here.
        Ok(vec![self.cert.clone()])
    }

    fn check(&mut self, structure: MessageStructure) -> sequoia_openpgp::Result<()> {
        // In this function, we implement our signature verification policy.

        let mut good = false;

        // we don't want compression and/or encryption
        if structure.len() > 1 || structure.is_empty() {
            bail!(
                "unexpected GPG message structure - expected plain signed data, got {} layers!",
                structure.len()
            );
        }
        let layer = &structure[0];
        let mut errors = Vec::new();
        match layer {
            MessageLayer::SignatureGroup { results } => {
                // We possibly have multiple signatures, but not all keys, so `or` all the individual results.
                for result in results {
                    match result {
                        Ok(_) => good = true,
                        Err(e) => errors.push(e),
                    }
                }
            }
            _ => return Err(anyhow::anyhow!("Unexpected message structure")),
        }

        if good {
            Ok(()) // Good signature.
        } else {
            for err in &errors {
                eprintln!("\t{err}");
            }
            Err(anyhow::anyhow!("encountered {} error(s)", errors.len()))
        }
    }
}
pub(crate) fn verify_signature<'msg>(
    msg: &'msg [u8],
    key: &[u8],
    detached_sig: Option<&[u8]>,
) -> Result<Vec<u8>, Error> {
    let cert = Cert::from_bytes(key)?;

    let policy = StandardPolicy::new();
    let helper = Helper { cert: &cert };

    let verified = if let Some(sig) = detached_sig {
        let mut verifier =
            DetachedVerifierBuilder::from_bytes(sig)?.with_policy(&policy, None, helper)?;
        verifier.verify_bytes(msg)?;
        msg.to_vec()
    } else {
        let mut verified = Vec::new();
        let mut verifier = VerifierBuilder::from_bytes(msg)?.with_policy(&policy, None, helper)?;
        let bytes = io::copy(&mut verifier, &mut verified)?;
        println!("{bytes} bytes verified");
        if !verifier.message_processed() {
            bail!("Failed to verify message!");
        }
        verified
    };

    Ok(verified)
}
