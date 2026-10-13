//! Root CA management: generate or load the CA used to sign per-host MITM
//! certificates, and check/install trust in the macOS System keychain.
//!
//! Port of httpmon's `internal/certutil`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use hudsucker::certificate_authority::RcgenAuthority;
use hudsucker::rcgen::{
    self, BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use hudsucker::rustls::crypto::aws_lc_rs;

pub const CA_COMMON_NAME: &str = "Project Proxy CA";
const KEY_FILE: &str = "ca-key.pem";
const CERT_PEM_FILE: &str = "ca-cert.pem";
const CERT_DER_FILE: &str = "ca-cert.cer";
const SYSTEM_KEYCHAIN: &str = "/Library/Keychains/System.keychain";

#[derive(Debug, thiserror::Error)]
pub enum CaError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("certificate: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("{0}")]
    Command(String),
}

/// The CA key + certificate, as stored on disk.
pub struct CaBundle {
    pub key_pem: String,
    pub cert_pem: String,
    pub cert_der: Vec<u8>,
    pub cert_pem_path: PathBuf,
    pub cert_der_path: PathBuf,
}

impl CaBundle {
    /// Loads the CA from `dir`, generating a fresh one if the files are
    /// missing or unreadable.
    pub fn load_or_create(dir: &Path) -> Result<Self, CaError> {
        fs::create_dir_all(dir)?;
        let key_path = dir.join(KEY_FILE);
        let pem_path = dir.join(CERT_PEM_FILE);
        let der_path = dir.join(CERT_DER_FILE);

        if let (Ok(key_pem), Ok(cert_pem)) =
            (fs::read_to_string(&key_path), fs::read_to_string(&pem_path))
            && let Ok(key) = KeyPair::from_pem(&key_pem)
            && Issuer::from_ca_cert_pem(&cert_pem, key).is_ok()
        {
            let cert_der = pem_to_der(&cert_pem)?;
            if !der_path.exists() {
                fs::write(&der_path, &cert_der)?;
            }
            return Ok(Self {
                key_pem,
                cert_pem,
                cert_der,
                cert_pem_path: pem_path,
                cert_der_path: der_path,
            });
        }

        let (key_pem, cert_pem, cert_der) = generate()?;
        write_private(&key_path, key_pem.as_bytes())?;
        fs::write(&pem_path, &cert_pem)?;
        fs::write(&der_path, &cert_der)?;
        Ok(Self {
            key_pem,
            cert_pem,
            cert_der,
            cert_pem_path: pem_path,
            cert_der_path: der_path,
        })
    }

    /// Builds the hudsucker authority that signs per-host leaf certificates.
    pub fn authority(&self) -> Result<RcgenAuthority, CaError> {
        let key = KeyPair::from_pem(&self.key_pem)?;
        let issuer = Issuer::from_ca_cert_pem(&self.cert_pem, key)?;
        Ok(RcgenAuthority::new(
            issuer,
            1_000,
            aws_lc_rs::default_provider(),
        ))
    }

    /// Uppercase hex SHA-1 fingerprint, matching `security -Z` output.
    pub fn sha1_fingerprint(&self) -> String {
        sha1_smol::Sha1::from(&self.cert_der)
            .digest()
            .to_string()
            .to_uppercase()
    }

    /// True when this exact certificate is in the System keychain (where
    /// `add-trusted-cert -d` puts it together with its trust settings).
    pub fn is_trusted(&self) -> bool {
        let out = Command::new("/usr/bin/security")
            .args([
                "find-certificate",
                "-a",
                "-Z",
                "-c",
                CA_COMMON_NAME,
                SYSTEM_KEYCHAIN,
            ])
            .output();
        match out {
            Ok(o) => fingerprint_listed(
                &String::from_utf8_lossy(&o.stdout),
                &self.sha1_fingerprint(),
            ),
            Err(_) => false,
        }
    }

    /// Adds the certificate to the System keychain as a trusted root.
    /// macOS shows an administrator password prompt.
    pub fn install_trust(&self) -> Result<(), CaError> {
        let cmd = format!(
            "/usr/bin/security add-trusted-cert -d -r trustRoot -k {} {}",
            shell_quote(SYSTEM_KEYCHAIN),
            shell_quote(&self.cert_der_path.to_string_lossy()),
        );
        run_as_admin(&cmd)
    }
}

fn generate() -> Result<(String, String, Vec<u8>), CaError> {
    let key = KeyPair::generate()?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, CA_COMMON_NAME);
    dn.push(DnType::OrganizationName, "Project Proxy");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2034, 1, 1);
    let cert = params.self_signed(&key)?;
    Ok((key.serialize_pem(), cert.pem(), cert.der().to_vec()))
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, CaError> {
    use base64::Engine as _;
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|e| CaError::Command(format!("invalid certificate PEM: {e}")))
}

fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents)
}

fn fingerprint_listed(security_output: &str, sha1: &str) -> bool {
    security_output.lines().any(|line| {
        line.trim()
            .strip_prefix("SHA-1 hash:")
            .is_some_and(|h| h.trim().eq_ignore_ascii_case(sha1))
    })
}

/// Single-quotes a string for `/bin/sh`.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Runs a shell command with administrator privileges via the standard macOS
/// authentication dialog.
pub(crate) fn run_as_admin(shell_cmd: &str) -> Result<(), CaError> {
    let escaped = shell_cmd.replace('\\', r"\\").replace('"', "\\\"");
    let script = format!("do shell script \"{escaped}\" with administrator privileges");
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(CaError::Command(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_then_reloads_same_ca() {
        let dir = tempfile::tempdir().unwrap();
        let a = CaBundle::load_or_create(dir.path()).unwrap();
        let b = CaBundle::load_or_create(dir.path()).unwrap();
        assert_eq!(a.cert_der, b.cert_der);
        assert_eq!(a.sha1_fingerprint().len(), 40);
        assert!(b.authority().is_ok());

        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(dir.path().join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn regenerates_when_files_are_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(KEY_FILE), "junk").unwrap();
        fs::write(dir.path().join(CERT_PEM_FILE), "junk").unwrap();
        let ca = CaBundle::load_or_create(dir.path()).unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn parses_security_fingerprint_output() {
        let out = "SHA-256 hash: AAAA\nSHA-1 hash: 0A1B2C\nkeychain: \"/Library/Keychains/System.keychain\"\n";
        assert!(fingerprint_listed(out, "0a1b2c"));
        assert!(!fingerprint_listed(out, "FFFF"));
    }

    #[test]
    fn quotes_for_shell() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
