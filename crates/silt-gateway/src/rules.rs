//! What the gateway lets through, and what it adds: pure, so its decisions
//! are tested without a network.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use silt_workspace::{Cidr, Network};

/// A credential the gateway adds to requests for `host`, under `path`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// A host, or `*.example.com` for its subdomains and itself.
    pub host: String,
    /// Requests whose path starts with it; `/` for all.
    pub path: String,
    pub header: String,
    /// The header's whole value, such as `Bearer …`. Never shown.
    pub value: String,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("host", &self.host)
            .field("path", &self.path)
            .field("header", &self.header)
            .finish_non_exhaustive()
    }
}

/// The gateway's rules for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub network: Network,
    /// Hosts "through Silt only" reaches; any public host when empty.
    pub hosts: Vec<String>,
    /// Ranges never reached, whatever a name resolves to.
    pub blocked: Vec<Cidr>,
    pub credentials: Vec<Credential>,
    /// GraphQL endpoints whose requests it reads.
    pub graphql: Vec<crate::GraphqlRule>,
    /// What each request must carry as its proxy's password, where others
    /// on this machine could reach the gateway too, as on macOS.
    pub proxy_token: Option<String>,
}

impl Rules {
    /// Whether `host` may be asked for at all.
    pub fn host_allowed(&self, host: &str) -> Result<(), String> {
        match self.network {
            Network::None => Err("this sandbox has no network".into()),
            Network::Gateway if !self.hosts.is_empty() && !self.hosts.iter().any(|pattern| matches(pattern, host)) => {
                Err(format!("{host} isn't among the hosts this workspace's sandboxes reach ({})", self.hosts.join(", ")))
            }
            _ => Ok(()),
        }
    }

    /// Whether an address a host resolved to may be connected to.
    pub fn address_allowed(&self, address: IpAddr) -> bool {
        let Some(address) = Cidr::parse(&address.to_string()) else { return false };
        !self.blocked.iter().any(|range| range.contains(address))
    }

    /// The credentials for a request to `host` at `path`.
    pub fn credentials_for<'a>(&'a self, host: &'a str, path: &'a str) -> impl Iterator<Item = &'a Credential> + 'a {
        self.credentials.iter().filter(move |credential| matches(&credential.host, host) && path.starts_with(&credential.path))
    }

    /// Whether the gateway reads requests to `host`, to add credentials: it
    /// passes everything else through unread.
    pub fn terminates(&self, host: &str) -> bool {
        self.credentials.iter().any(|credential| matches(&credential.host, host))
            || self.graphql.iter().any(|rule| rule.host.eq_ignore_ascii_case(host))
    }

    /// The GraphQL rule for a request to `host` at `path`.
    pub fn graphql_for(&self, host: &str, path: &str) -> Option<&crate::GraphqlRule> {
        self.graphql.iter().find(|rule| rule.covers(host, path))
    }
}

/// Whether `host` is `pattern`, or under it for `*.` patterns.
pub fn matches(pattern: &str, host: &str) -> bool {
    let (pattern, host) = (pattern.to_ascii_lowercase(), host.trim_end_matches('.').to_ascii_lowercase());
    match pattern.strip_prefix("*.") {
        Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
        None => host == pattern,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(network: Network) -> Rules {
        Rules {
            network,
            hosts: vec!["*.github.com".into(), "registry.npmjs.org".into()],
            blocked: silt_workspace::subtract(&silt_workspace::ALWAYS_BLOCKED.map(str::to_owned), &["10.20.0.0/16".into()]),
            credentials: vec![Credential {
                host: "api.github.com".into(),
                path: "/repos/acme/".into(),
                header: "authorization".into(),
                value: "Bearer s3cret".into(),
            }],
            graphql: Vec::new(),
            proxy_token: None,
        }
    }

    #[test]
    fn through_silt_only_reaches_its_hosts() {
        let gateway = rules(Network::Gateway);
        assert!(gateway.host_allowed("api.github.com").is_ok() && gateway.host_allowed("github.com").is_ok());
        assert!(gateway.host_allowed("registry.npmjs.org").is_ok());
        assert!(gateway.host_allowed("evil.example").unwrap_err().contains("isn't among the hosts"));
        assert!(rules(Network::Full).host_allowed("evil.example").is_ok());
        assert!(rules(Network::None).host_allowed("github.com").is_err());
    }

    #[test]
    fn this_machine_its_networks_and_metadata_are_never_reached() {
        let full = rules(Network::Full);
        for address in ["127.0.0.1", "10.0.0.5", "169.254.169.254", "192.168.1.1", "::1", "fd00::1"] {
            assert!(!full.address_allowed(address.parse().unwrap()), "{address}");
        }
        assert!(full.address_allowed("140.82.112.3".parse().unwrap()));
        assert!(full.address_allowed("10.20.4.12".parse().unwrap()), "an allowed range");
    }

    #[test]
    fn credentials_go_only_where_they_belong() {
        let full = rules(Network::Full);
        assert_eq!(full.credentials_for("api.github.com", "/repos/acme/api").count(), 1);
        assert_eq!(full.credentials_for("api.github.com", "/repos/other/x").count(), 0);
        assert_eq!(full.credentials_for("evil.example", "/repos/acme/api").count(), 0);
        assert!(full.terminates("API.GitHub.com") && !full.terminates("github.com"));
        assert!(!format!("{:?}", full.credentials[0]).contains("s3cret"), "never in a log");
    }
}
