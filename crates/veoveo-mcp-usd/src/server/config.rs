//! The server's configuration. Every flag has its environment variable. The Hub
//! issues the database login and the access-token trust when the server registers, so
//! neither is configured here.
use std::path::PathBuf;

use clap::Parser;
use secrecy::SecretString;
use url::Url;
use veoveo_hub_sdk::{PublicDeployment, parse_allowed_host_authority};

#[derive(Parser)]
#[command(name = "veoveo-mcp-usd", about = "USD MCP server")]
pub(super) struct Args {
    /// Print native public metadata without Hub, database or telemetry startup.
    #[arg(long)]
    pub describe_registration: bool,
    #[arg(long, env = "VEOVEO_DEPLOYMENT_ID")]
    pub deployment_id: veoveo_hub_sdk::DeploymentId,
    #[arg(long, env = "VEOVEO_PORT", default_value_t = 8806)]
    pub port: u16,
    /// The origin where the Hub reaches the server; it serves MCP at
    /// `{public_base_url}/usd/mcp` and registers that URL.
    #[arg(long, env = "VEOVEO_PUBLIC_BASE_URL")]
    pub public_base_url: String,
    /// The Hub's RFC 8414 issuer; registration discovers its endpoint from there.
    #[arg(
        long,
        env = "VEOVEO_HUB_ISSUER",
        required_unless_present = "describe_registration"
    )]
    pub hub_issuer: Option<Url>,
    /// The installation's registration secret, or the placeholder the installation's edge
    /// replaces.
    #[arg(
        long,
        env = "VEOVEO_REGISTRATION_SECRET",
        required_unless_present = "describe_registration",
        hide_env_values = true,
        value_parser = parse_secret
    )]
    pub registration_secret: Option<SecretString>,
    /// Private deployment key and saved native registration retained across process restarts.
    #[arg(
        long,
        env = "VEOVEO_KEY_DIRECTORY",
        default_value = "/var/lib/veoveo/usd/keys"
    )]
    pub key_directory: PathBuf,
    #[arg(long, env = "VEOVEO_ALLOW_LOOPBACK_HOSTS", default_value_t = false)]
    pub allow_loopback_hosts: bool,
    #[arg(
        long = "allowed-host",
        env = "VEOVEO_ALLOWED_HOSTS",
        value_name = "HOST",
        value_delimiter = ',',
        value_parser = parse_allowed_host
    )]
    pub allowed_hosts: Vec<String>,
}

impl Args {
    pub fn public_deployment(&self) -> anyhow::Result<PublicDeployment> {
        PublicDeployment::new(&self.public_base_url)
    }
}

fn parse_secret(value: &str) -> Result<SecretString, String> {
    (!value.is_empty())
        .then(|| SecretString::from(value))
        .ok_or_else(|| "secret must not be empty".to_owned())
}

fn parse_allowed_host(value: &str) -> Result<String, String> {
    let value = value.trim();
    parse_allowed_host_authority(value)
        .map(|_| value.to_owned())
        .ok_or_else(|| "expected a host authority such as vv-mcp-usd.internal:8806".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hub_and_the_secret_are_required() {
        let base = [
            "veoveo-mcp-usd",
            "--deployment-id",
            "019a5b30-0000-7000-8000-000000000001",
            "--public-base-url",
            "http://vv-mcp-usd.internal:8806",
        ];
        assert!(Args::try_parse_from(base).is_err());
        let args = Args::try_parse_from(base.into_iter().chain([
            "--hub-issuer",
            "http://vv-hub.internal:8080",
            "--registration-secret",
            "placeholder",
            "--allowed-host",
            "vv-mcp-usd.internal:8806",
        ]))
        .unwrap();
        assert_eq!(args.port, 8806);
        assert_eq!(args.allowed_hosts, ["vv-mcp-usd.internal:8806"]);
        assert!(args.public_deployment().is_ok());
    }
    #[test]
    fn offline_description_requires_no_hub_or_admission_credentials() {
        let args = Args::try_parse_from([
            "veoveo-mcp-usd",
            "--describe-registration",
            "--deployment-id",
            "019a5b30-0000-7000-8000-000000000001",
            "--public-base-url",
            "https://usd.example",
        ])
        .unwrap();
        assert!(args.describe_registration);
        assert!(args.hub_issuer.is_none());
        assert!(args.registration_secret.is_none());
        assert!(
            Args::try_parse_from([
                "veoveo-mcp-usd",
                "--describe-registration",
                "--public-base-url",
                "https://usd.example"
            ])
            .is_err()
        );
    }
}
