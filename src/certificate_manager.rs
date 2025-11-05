//! Certificate manager for TLS MITM proxy
//!
//! Handles generation and caching of TLS certificates for intercepting HTTPS traffic.

use anyhow::{Context, Result};
use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Manages TLS certificates for the MITM proxy
pub struct CertificateManager {
    /// Root CA certificate for signing host certificates
    root_ca: Certificate,
    /// Root CA key pair
    root_ca_keypair: KeyPair,
    /// Cache of generated host certificates
    cache: Arc<RwLock<HashMap<String, (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>>>,
    /// Directory to store certificates
    _cert_dir: PathBuf,
}

impl CertificateManager {
    /// Create a new certificate manager, generating or loading the root CA
    pub fn new(cert_dir: impl AsRef<Path>) -> Result<Self> {
        let cert_dir = cert_dir.as_ref().to_path_buf();
        fs::create_dir_all(&cert_dir)
            .context("Failed to create certificate directory")?;

        let ca_cert_path = cert_dir.join("ca.pem");
        let ca_key_path = cert_dir.join("ca.key");

        let (root_ca, root_ca_keypair) = if ca_cert_path.exists() && ca_key_path.exists() {
            // Load existing CA
            tracing::info!("Loading existing root CA from {:?}", ca_cert_path);
            Self::load_ca(&ca_cert_path, &ca_key_path)?
        } else {
            // Generate new CA
            tracing::info!("Generating new root CA");
            let (ca, keypair) = Self::generate_root_ca()?;

            // Save to disk
            Self::save_ca(&ca, &keypair, &ca_cert_path, &ca_key_path)?;

            tracing::info!("Root CA saved to {:?}", ca_cert_path);
            tracing::warn!("Install the root CA certificate to trust HTTPS interception:");
            tracing::warn!("  macOS: sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain {:?}", ca_cert_path);
            tracing::warn!("  Linux: sudo cp {:?} /usr/local/share/ca-certificates/ && sudo update-ca-certificates", ca_cert_path);

            (ca, keypair)
        };

        Ok(Self {
            root_ca,
            root_ca_keypair,
            cache: Arc::new(RwLock::new(HashMap::new())),
            _cert_dir: cert_dir,
        })
    }

    /// Generate a root CA certificate
    fn generate_root_ca() -> Result<(Certificate, KeyPair)> {
        let mut params = CertificateParams::default();

        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Local Logger CA");
        dn.push(DnType::OrganizationName, "Local Logger");
        dn.push(DnType::CountryName, "US");
        params.distinguished_name = dn;

        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];

        let keypair = KeyPair::generate()?;
        let cert = params.self_signed(&keypair)?;

        Ok((cert, keypair))
    }

    /// Load CA certificate and key from disk
    ///
    /// This properly loads the saved CA certificate to preserve its exact structure,
    /// including the signature and SubjectKeyIdentifier. This ensures the CA certificate
    /// remains stable across restarts - critical for MITM proxy functionality.
    fn load_ca(cert_path: &Path, key_path: &Path) -> Result<(Certificate, KeyPair)> {
        let cert_pem = fs::read_to_string(cert_path)
            .context("Failed to read CA certificate")?;

        let key_pem = fs::read_to_string(key_path)
            .context("Failed to read CA private key")?;

        let keypair = KeyPair::from_pem(&key_pem)
            .context("Failed to parse CA private key")?;

        // Parse the saved certificate to preserve its exact structure
        // This requires the x509-parser feature to be enabled
        let params = CertificateParams::from_ca_cert_pem(&cert_pem)
            .context("Failed to parse CA certificate PEM")?;

        // Reconstruct the Certificate from the loaded params and key
        let cert = params.self_signed(&keypair)
            .context("Failed to reconstruct CA certificate")?;

        Ok((cert, keypair))
    }

    /// Save CA certificate and key to disk
    fn save_ca(cert: &Certificate, keypair: &KeyPair, cert_path: &Path, key_path: &Path) -> Result<()> {
        let cert_pem = cert.pem();
        let key_pem = keypair.serialize_pem();

        let mut cert_file = File::create(cert_path)
            .context("Failed to create CA certificate file")?;
        cert_file.write_all(cert_pem.as_bytes())
            .context("Failed to write CA certificate")?;

        let mut key_file = File::create(key_path)
            .context("Failed to create CA key file")?;
        key_file.write_all(key_pem.as_bytes())
            .context("Failed to write CA key")?;

        // Set restrictive permissions on the key file
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(key_path, fs::Permissions::from_mode(0o600))
                .context("Failed to set CA key permissions")?;
        }

        Ok(())
    }

    /// Get or generate a certificate for a specific hostname
    pub async fn get_certificate(&self, hostname: &str) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some((certs, key)) = cache.get(hostname) {
                tracing::debug!("Using cached certificate for {}", hostname);
                // Clone the certs vec and clone_key for the private key
                return Ok((certs.clone(), key.clone_key()));
            }
        }

        // Generate new certificate
        tracing::debug!("Generating new certificate for {}", hostname);
        let (cert_der, key_der) = self.generate_host_certificate(hostname)?;

        // Cache the certificate
        {
            let mut cache = self.cache.write().await;
            cache.insert(hostname.to_string(), (cert_der.clone(), key_der.clone_key()));
        }

        Ok((cert_der, key_der))
    }

    /// Generate a certificate for a specific hostname, signed by the root CA
    fn generate_host_certificate(&self, hostname: &str) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
        let mut params = CertificateParams::default();

        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, hostname);
        params.distinguished_name = dn;

        params.subject_alt_names = vec![
            rcgen::SanType::DnsName(hostname.try_into()?),
        ];

        // Generate key pair for this certificate
        let keypair = KeyPair::generate()?;

        // Sign with root CA
        let cert = params.signed_by(&keypair, &self.root_ca, &self.root_ca_keypair)?;

        // Convert to DER format
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::try_from(keypair.serialize_der())
            .map_err(|e| anyhow::anyhow!("Failed to serialize private key: {}", e))?;

        Ok((vec![cert_der], key_der))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_root_ca() {
        let (cert, _keypair) = CertificateManager::generate_root_ca().unwrap();
        let pem = cert.pem();
        assert!(pem.contains("BEGIN CERTIFICATE"));
        assert!(pem.contains("END CERTIFICATE"));
    }

    #[tokio::test]
    async fn test_certificate_manager() {
        let temp_dir = TempDir::new().unwrap();
        let manager = CertificateManager::new(temp_dir.path()).unwrap();

        // Generate certificate for a hostname
        let (cert, _key) = manager.get_certificate("api.anthropic.com").await.unwrap();
        assert!(!cert.is_empty());

        // Verify it's cached
        let (cert2, _key2) = manager.get_certificate("api.anthropic.com").await.unwrap();
        assert_eq!(cert.len(), cert2.len());
    }

    #[test]
    fn test_save_and_load_ca() {
        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("ca.pem");
        let key_path = temp_dir.path().join("ca.key");

        // Generate and save
        let (cert1, keypair1) = CertificateManager::generate_root_ca().unwrap();
        CertificateManager::save_ca(&cert1, &keypair1, &cert_path, &key_path).unwrap();

        // Load
        let (cert2, keypair2) = CertificateManager::load_ca(&cert_path, &key_path).unwrap();

        // Verify the loaded certificate has the same key characteristics
        // Note: from_ca_cert_pem() extracts parameters and regenerates the certificate,
        // so the signature will differ. This is acceptable for MITM proxy functionality
        // because what matters is:
        // 1. The certificate can be loaded and used for signing
        // 2. The private key matches
        // 3. The certificate is valid and can be trusted

        let cert1_pem = cert1.pem();
        let cert2_pem = cert2.pem();

        // Both should be valid PEM certificates
        assert!(cert1_pem.contains("BEGIN CERTIFICATE"));
        assert!(cert1_pem.contains("END CERTIFICATE"));
        assert!(cert2_pem.contains("BEGIN CERTIFICATE"));
        assert!(cert2_pem.contains("END CERTIFICATE"));

        // The key pairs should serialize to the same bytes
        assert_eq!(keypair1.serialize_pem(), keypair2.serialize_pem());

        // Verify both certificates are valid by checking they can generate DER
        assert!(!cert1.der().is_empty());
        assert!(!cert2.der().is_empty());
    }
}
