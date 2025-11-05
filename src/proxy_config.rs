//! Configuration for the proxy server

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: IpAddr,

    #[serde(default = "default_listen_port")]
    pub listen_port: u16,

    #[serde(default)]
    pub tls: TlsConfig,

    #[serde(default)]
    pub recording: RecordingConfig,

    #[serde(default)]
    pub filtering: FilteringConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    #[serde(default = "default_cert_dir")]
    pub cert_dir: PathBuf,

    #[serde(default = "default_true")]
    pub generate_ca: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: PathBuf,

    #[serde(default = "default_true")]
    pub pretty_print: bool,

    #[serde(default = "default_true")]
    pub include_bodies: bool,

    #[serde(default = "default_max_body_size")]
    pub max_body_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilteringConfig {
    #[serde(default = "default_target_hosts")]
    pub target_hosts: Vec<String>,

    #[serde(default)]
    pub capture_patterns: Vec<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            listen_port: default_listen_port(),
            tls: TlsConfig::default(),
            recording: RecordingConfig::default(),
            filtering: FilteringConfig::default(),
        }
    }
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            cert_dir: default_cert_dir(),
            generate_ca: true,
        }
    }
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            pretty_print: true,
            include_bodies: true,
            max_body_size: default_max_body_size(),
        }
    }
}

impl Default for FilteringConfig {
    fn default() -> Self {
        Self {
            target_hosts: default_target_hosts(),
            capture_patterns: vec![],
        }
    }
}

impl ProxyConfig {
    /// Load configuration from a TOML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let contents = std::fs::read_to_string(path.as_ref())
            .context("Failed to read configuration file")?;
        let config: ProxyConfig = toml::from_str(&contents)
            .context("Failed to parse configuration file")?;
        Ok(config)
    }

    /// Load configuration from environment variables or use defaults
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(addr) = std::env::var("CLAUDE_LOGGER_PROXY_ADDR") {
            if let Ok(ip) = addr.parse() {
                config.listen_addr = ip;
            }
        }

        if let Ok(port) = std::env::var("CLAUDE_LOGGER_PROXY_PORT") {
            if let Ok(p) = port.parse() {
                config.listen_port = p;
            }
        }

        if let Ok(dir) = std::env::var("CLAUDE_LOGGER_PROXY_CERT_DIR") {
            config.tls.cert_dir = PathBuf::from(dir);
        }

        if let Ok(dir) = std::env::var("CLAUDE_MCP_LOCAL_LOGGER_DIR") {
            config.recording.output_dir = PathBuf::from(dir);
        }

        config
    }

    /// Save configuration to a TOML file
    #[allow(dead_code)]
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let contents = toml::to_string_pretty(self)
            .context("Failed to serialize configuration")?;
        std::fs::write(path.as_ref(), contents)
            .context("Failed to write configuration file")?;
        Ok(())
    }
}

fn default_listen_addr() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

fn default_listen_port() -> u16 {
    6969
}

fn default_cert_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local-logger").join("certs")
}

fn default_output_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local-logger")
}

fn default_max_body_size() -> usize {
    10 * 1024 * 1024 // 10MB
}

fn default_target_hosts() -> Vec<String> {
    vec!["api.anthropic.com".to_string()]
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = ProxyConfig::default();
        assert_eq!(config.listen_port, 6969);
        assert!(config.recording.include_bodies);
        assert!(config.tls.generate_ca);
    }

    #[test]
    fn test_save_and_load_config() {
        let config = ProxyConfig::default();
        let temp_file = NamedTempFile::new().unwrap();

        config.save(temp_file.path()).unwrap();
        let loaded = ProxyConfig::from_file(temp_file.path()).unwrap();

        assert_eq!(config.listen_port, loaded.listen_port);
        assert_eq!(config.recording.max_body_size, loaded.recording.max_body_size);
    }

    #[test]
    fn test_from_env() {
        std::env::set_var("CLAUDE_LOGGER_PROXY_PORT", "9090");
        let config = ProxyConfig::from_env();
        assert_eq!(config.listen_port, 9090);
        std::env::remove_var("CLAUDE_LOGGER_PROXY_PORT");
    }
}
