//! Pure-Rust TLS authority used to terminate guest `CONNECT` tunnels.
//!
//! The browser cannot expose a raw socket, so HTTPS proxying has to decrypt the
//! guest's TLS locally and hand the resulting HTTP request to `fetch()`. This
//! module owns one ephemeral CA and leaf signing key, mints a hostname-specific
//! certificate on demand, and creates rustls server sessions for the tunnel.

use ed25519_dalek::pkcs8::EncodePrivateKey;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyIdMethod, KeyUsagePurpose, PublicKeyData, SerialNumber, SignatureAlgorithm,
    SigningKey, PKCS_ED25519,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::Digest;
use std::collections::HashMap;
use std::sync::Arc;

struct EdKey {
    signing: ed25519_dalek::SigningKey,
    public: [u8; 32],
    pkcs8: Vec<u8>,
}

impl EdKey {
    fn generate() -> Result<Self, String> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).map_err(|e| format!("entropy: {e}"))?;
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        let public = *signing.verifying_key().as_bytes();
        let pkcs8 = signing
            .to_pkcs8_der()
            .map_err(|e| format!("PKCS#8: {e}"))?
            .as_bytes()
            .to_vec();
        Ok(Self {
            signing,
            public,
            pkcs8,
        })
    }

    fn key_id(&self) -> Vec<u8> {
        sha2::Sha256::digest(self.public).to_vec()
    }

    fn serial(&self) -> SerialNumber {
        let mut serial = self.key_id()[..20].to_vec();
        serial[0] &= 0x7f;
        serial.into()
    }
}

impl PublicKeyData for EdKey {
    fn der_bytes(&self) -> &[u8] {
        &self.public
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ED25519
    }
}

impl SigningKey for EdKey {
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        use ed25519_dalek::Signer;
        Ok(self.signing.sign(msg).to_bytes().to_vec())
    }
}

/// An ephemeral MITM CA plus hostname-specific rustls configuration cache.
pub struct TlsAuthority {
    ca_params: CertificateParams,
    ca_key: EdKey,
    ca_der: Vec<u8>,
    leaf_key: EdKey,
    configs: HashMap<String, Arc<rustls::ServerConfig>>,
}

impl TlsAuthority {
    pub fn new() -> Result<Self, String> {
        let ca_key = EdKey::generate()?;
        let leaf_key = EdKey::generate()?;

        let mut distinguished_name = DistinguishedName::new();
        distinguished_name.push(DnType::CommonName, "rv64.js ephemeral proxy CA");
        let mut ca_params = CertificateParams::default();
        ca_params.serial_number = Some(ca_key.serial());
        ca_params.distinguished_name = distinguished_name;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::CrlSign,
        ];
        ca_params.key_identifier_method = KeyIdMethod::PreSpecified(ca_key.key_id());
        let ca_der = ca_params
            .self_signed(&ca_key)
            .map_err(|e| format!("CA certificate: {e}"))?
            .der()
            .to_vec();

        Ok(Self {
            ca_params,
            ca_key,
            ca_der,
            leaf_key,
            configs: HashMap::new(),
        })
    }

    pub fn ca_der(&self) -> &[u8] {
        &self.ca_der
    }

    pub fn server(&mut self, host: &str) -> Result<rustls::ServerConnection, String> {
        let config = match self.configs.get(host) {
            Some(config) => Arc::clone(config),
            None => {
                let config = self.build_config(host)?;
                self.configs.insert(host.to_string(), Arc::clone(&config));
                config
            }
        };
        rustls::ServerConnection::new(config).map_err(|e| format!("TLS server: {e}"))
    }

    fn build_config(&self, host: &str) -> Result<Arc<rustls::ServerConfig>, String> {
        let mut distinguished_name = DistinguishedName::new();
        distinguished_name.push(DnType::CommonName, host);
        let mut params =
            CertificateParams::new(vec![host.to_string()]).map_err(|e| format!("SAN: {e}"))?;
        params.serial_number = Some(self.leaf_key.serial());
        params.distinguished_name = distinguished_name;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        params.key_identifier_method = KeyIdMethod::PreSpecified(self.leaf_key.key_id());

        let issuer = Issuer::from_params(&self.ca_params, &self.ca_key);
        let leaf_der = params
            .signed_by(&self.leaf_key, &issuer)
            .map_err(|e| format!("leaf certificate: {e}"))?
            .der()
            .to_vec();

        let provider = Arc::new(oxitls_rustcrypto_provider::provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS versions: {e}"))?
            .with_no_client_auth()
            .with_single_cert(
                vec![
                    CertificateDer::from(leaf_der),
                    CertificateDer::from(self.ca_der.clone()),
                ],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.leaf_key.pkcs8.clone())),
            )
            .map_err(|e| format!("TLS certificate/key: {e}"))?;
        Ok(Arc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mints_a_server_config_and_reuses_it() {
        let mut ca = TlsAuthority::new().unwrap();
        assert!(ca.ca_der().len() > 200);
        let a = ca.server("example.test").unwrap();
        let b = ca.server("example.test").unwrap();
        assert!(a.is_handshaking());
        assert!(b.is_handshaking());
        assert_eq!(ca.configs.len(), 1);
    }
}
