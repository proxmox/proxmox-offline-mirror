use anyhow::{bail, Error};

use sequoia_openpgp::{
    parse::{
        stream::{
            DetachedVerifierBuilder, MessageLayer, MessageStructure, VerificationError,
            VerificationHelper, VerifierBuilder,
        },
        Parse,
    },
    policy::StandardPolicy,
    types::HashAlgorithm,
    Cert, KeyHandle,
};
use std::io;

use crate::config::WeakCryptoConfig;

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
            if errors.len() > 1 {
                eprintln!("\nEncountered {} errors:", errors.len());
            }

            for (n, err) in errors.iter().enumerate() {
                if errors.len() > 1 {
                    eprintln!("\nSignature #{n}: {err}");
                } else {
                    eprintln!("\n{err}");
                }
                match err {
                    VerificationError::MalformedSignature { error, .. }
                    | VerificationError::UnboundKey { error, .. }
                    | VerificationError::BadKey { error, .. }
                    | VerificationError::BadSignature { error, .. } => {
                        let mut cause = error.chain();
                        if cause.len() > 1 {
                            cause.next(); // already included in `err` above
                            eprintln!("Caused by:");
                            for (n, e) in cause.enumerate() {
                                eprintln!("\t{n}: {e}");
                            }
                        }
                    }
                    VerificationError::MissingKey { .. } => {} // doesn't contain a cause
                };
            }
            eprintln!();
            Err(anyhow::anyhow!("No valid signature found."))
        }
    }
}

/// Verifies GPG-signed `msg` was signed by `key`, returning the verified data without signature.
pub(crate) fn verify_signature<'msg>(
    msg: &'msg [u8],
    key: &[u8],
    detached_sig: Option<&[u8]>,
    weak_crypto: &WeakCryptoConfig,
) -> Result<Vec<u8>, Error> {
    let cert = Cert::from_bytes(key)?;

    let mut policy = StandardPolicy::new();
    if weak_crypto.allow_sha1 {
        policy.accept_hash(HashAlgorithm::SHA1);
    }
    if let Some(min_dsa) = weak_crypto.min_dsa_key_size {
        if min_dsa <= 1024 {
            policy.accept_asymmetric_algo(sequoia_openpgp::policy::AsymmetricAlgorithm::DSA1024);
        }
    }
    if let Some(min_rsa) = weak_crypto.min_dsa_key_size {
        if min_rsa <= 1024 {
            policy.accept_asymmetric_algo(sequoia_openpgp::policy::AsymmetricAlgorithm::RSA1024);
        }
    }

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
