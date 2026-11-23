//! HTTP client for untrusted URLs (saved links, their redirects). It only
//! connects to public addresses, so a link cannot reach the home LAN or
//! services listening on this host (SSRF).
//!
//! Two checks, because neither covers everything alone:
//! - the DNS resolver drops private addresses a hostname resolves to;
//! - IP-literal URLs (`http://192.168.1.1/`) never reach the resolver, so
//!   every request URL and every redirect target is checked directly.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use thiserror::Error;
use url::{Host, Url};

const MAX_REDIRECTS: usize = 10;

/// Which addresses the client may connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addresses {
    /// Production: public internet only.
    PublicOnly,
    /// Tests: also loopback, where wiremock listens. Other private
    /// ranges stay blocked, so tests can still prove the guard works.
    AllowLoopback,
}

impl Addresses {
    fn allows(self, ip: IpAddr) -> bool {
        // Loopback first: tests never reach `is_public` for 127.0.0.1.
        (self == Addresses::AllowLoopback && ip.is_loopback()) || is_public(ip)
    }
}

/// A URL pointed at an address the policy forbids.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("blocked address {0}")]
pub struct Blocked(pub IpAddr);

/// True for addresses on the public internet. False for loopback, private
/// (RFC 1918), link-local, CGNAT, unspecified, multicast, documentation
/// ranges and IPv6 equivalents, including IPv4 addresses embedded in IPv6.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => is_public_ipv4(addr),
        IpAddr::V6(addr) => is_public_ipv6(addr),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    if ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || ip.is_link_local()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_unspecified()
    {
        false
    } else {
        let octets = ip.octets();
        match octets {
            [0, ..] => false,
            [100, b, ..] if (b & 0b1100_0000) == 64 => false,
            [192, 0, 0, d] if d != 9 && d != 10 => false,
            [198, 18..=19, _, _] => false,
            [240..=255, ..] => false,
            _ => true,
        }
    }
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    match embedded_ipv4(ip) {
        Some(a) => is_public_ipv4(a),
        None => {
            !(ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unicast_link_local()
                || ip.is_unique_local()
                || ip.is_unspecified()
                || ip.segments()[0] & 0xffc0 == 0xfec0
                || matches!(ip.segments(), [0x2001, 0xdb8, ..]))
        }
    }
}

fn embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    ip.to_ipv4_mapped().or_else(|| match ip.segments() {
        [0x64, 0xff9b, 0, 0, 0, 0, hi, lo] | [0x2002, hi, lo, ..] | [0, 0, 0, 0, 0, 0, hi, lo] => {
            Some(Ipv4Addr::from_bits((u32::from(hi) << 16) | u32::from(lo)))
        }
        _ => None,
    })
}

/// Rejects URLs whose host is an IP literal the policy forbids. Hostnames
/// pass here; the resolver checks what they resolve to.
pub fn check_url(url: &Url, policy: Addresses) -> Result<(), Blocked> {
    let ip = match url.host() {
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip),
        Some(Host::Domain(_)) | None => return Ok(()),
    };
    if policy.allows(ip) {
        Ok(())
    } else {
        Err(Blocked(ip))
    }
}

/// `reqwest::Client` that can only reach allowed addresses. A newtype, so
/// code that fetches untrusted URLs cannot be handed an unguarded client.
#[derive(Debug, Clone)]
pub struct WebClient {
    http: reqwest::Client,
    policy: Addresses,
}

impl WebClient {
    pub fn new(policy: Addresses) -> reqwest::Result<WebClient> {
        let http = reqwest::Client::builder()
            .dns_resolver(Arc::new(GuardedResolver(policy)))
            .redirect(Policy::custom(move |attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    return attempt.error("too many redirects");
                }
                // Decide first: `attempt.url()` borrows `attempt`, and
                // `follow`/`error` consume it.
                match check_url(attempt.url(), policy) {
                    Ok(()) => attempt.follow(),
                    Err(blocked) => attempt.error(blocked),
                }
            }))
            // A proxy would resolve names itself, bypassing the resolver.
            .no_proxy()
            .build()?;
        Ok(WebClient { http, policy })
    }

    /// Starts a request after checking the URL itself.
    pub fn request(
        &self,
        method: reqwest::Method,
        url: &Url,
    ) -> Result<reqwest::RequestBuilder, Blocked> {
        check_url(url, self.policy)?;
        Ok(self.http.request(method, url.clone()))
    }
}

/// System DNS, minus the addresses the policy forbids.
struct GuardedResolver(Addresses);

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let policy = self.0;
        // `Resolving` is a boxed future: `Box::pin(async move { … })`.
        Box::pin(async move {
            let host = name.as_str();
            let (allowed, rejected): (Vec<SocketAddr>, Vec<SocketAddr>) =
                tokio::net::lookup_host((host, 0))
                    .await?
                    .partition(|addr| policy.allows(addr.ip()));
            if allowed.is_empty() {
                // `Blocked` (not a string) so `FetchError` can recognize it
                // in the source chain and not retry.
                if let Some(addr) = rejected.first() {
                    return Err(Blocked(addr.ip()).into());
                }
                return Err(format!("{host} has no addresses").into());
            }
            let addrs: Addrs = Box::new(allowed.into_iter());
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addresses_are_public() {
        for a in [
            "1.1.1.1",
            "93.184.215.14",
            "2606:4700:4700::1111",
            "64:ff9b::808:808", // NAT64 of 8.8.8.8: the embedded IPv4 decides
            "2002:808:808::1",  // 6to4 of 8.8.8.8
        ] {
            assert!(is_public(ip(a)), "{a}");
        }
    }

    #[test]
    fn local_and_reserved_addresses_are_not_public() {
        let blocked = [
            "127.0.0.1",          // loopback
            "10.1.2.3",           // RFC 1918
            "172.16.0.1",         // RFC 1918
            "192.168.1.1",        // RFC 1918: the home router
            "169.254.169.254",    // link-local
            "100.64.0.1",         // CGNAT, RFC 6598
            "0.0.0.0",            // unspecified
            "224.0.0.1",          // multicast
            "255.255.255.255",    // broadcast
            "192.0.2.1",          // documentation, RFC 5737
            "::1",                // IPv6 loopback
            "::",                 // IPv6 unspecified
            "fe80::1",            // IPv6 link-local
            "fd00::1",            // IPv6 unique local (ULA)
            "ff02::1",            // IPv6 multicast
            "::ffff:127.0.0.1",   // IPv4-mapped loopback
            "::ffff:192.168.1.1", // IPv4-mapped private
            "198.18.0.1",         // benchmarking, RFC 2544
            "240.0.0.1",          // reserved, RFC 1112
            "::7f00:1",           // IPv4-compatible 127.0.0.1 (deprecated form)
            "64:ff9b::c0a8:101",  // NAT64 of 192.168.1.1, RFC 6052
            "2002:c0a8:101::1",   // 6to4 of 192.168.1.1, RFC 3056
            "fec0::1",            // site-local (deprecated)
            "2001:db8::1",        // documentation, RFC 3849
        ];
        for a in blocked {
            assert!(!is_public(ip(a)), "{a}");
        }
    }

    #[test]
    fn check_url_blocks_private_ip_literals() {
        let url = Url::parse("http://192.168.1.1/admin").unwrap();
        assert_eq!(
            check_url(&url, Addresses::PublicOnly),
            Err(Blocked(ip("192.168.1.1")))
        );
    }

    #[test]
    fn check_url_allows_loopback_only_when_asked() {
        let url = Url::parse("http://127.0.0.1:8080/").unwrap();
        assert!(check_url(&url, Addresses::AllowLoopback).is_ok());
        // Hostnames are left to the resolver.
        let named = Url::parse("http://localhost/").unwrap();
        assert!(check_url(&named, Addresses::PublicOnly).is_ok());
    }
}
