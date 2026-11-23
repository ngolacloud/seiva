//! A conservative, dependency-free egress guard for a tenant-supplied URL.
//!
//! A webhook endpoint is attacker-influenced data: left unchecked it is a
//! Server-Side Request Forgery (SSRF) vector — a tenant pointing it at
//! `http://169.254.169.254/…` to read cloud-metadata credentials, or at an
//! internal `10.x` service. [`validate_egress`] is the in-engine check run before
//! every POST until ADR 0045 lands a shared primitive: require TLS, and reject a
//! destination whose host is loopback, link-local (incl. the metadata address),
//! RFC 1918 / unique-local private space, or otherwise non-routable.
//!
//! It is deliberately conservative and **does not resolve DNS** — a hostname that
//! resolves to a private address slips past the literal-IP check here; the DNS-time
//! guard is ADR 0045's job. What it does catch is the direct-IP and obvious-name
//! cases without pulling a URL/DNS dependency into a signing crate.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Why a destination URL was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EgressError {
    /// The URL is not `https://` — a webhook is delivered over TLS (ADR 0033).
    #[error("webhook endpoint must be https")]
    NotHttps,
    /// The URL carried no host to check.
    #[error("webhook endpoint has no host")]
    NoHost,
    /// The host is a loopback / link-local / private / metadata address (or the
    /// `localhost` name) — a non-routable SSRF target.
    #[error("webhook endpoint resolves to a blocked (private/loopback/metadata) address")]
    Blocked,
}

/// Validate a destination URL before an outbound POST.
///
/// # Errors
///
/// - [`EgressError::NotHttps`] if the scheme is not `https`.
/// - [`EgressError::NoHost`] if there is no host component.
/// - [`EgressError::Blocked`] if the host is `localhost`/`*.localhost`, or an IP
///   literal in a loopback, link-local (incl. `169.254.169.254`), private
///   (RFC 1918 / unique-local), unspecified or otherwise non-global range.
pub fn validate_egress(url: &str) -> Result<(), EgressError> {
    // TLS only. Case-insensitive on the scheme, as URLs allow.
    let rest = url
        .strip_prefix("https://")
        .or_else(|| strip_prefix_ci(url, "https://"))
        .ok_or(EgressError::NotHttps)?;

    let host = host_of(rest).ok_or(EgressError::NoHost)?;
    if host.is_empty() {
        return Err(EgressError::NoHost);
    }

    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Err(EgressError::Blocked);
    }

    // Only an IP *literal* is range-checked here; a name is left to ADR 0045's
    // DNS-time guard (documented limitation).
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(EgressError::Blocked);
        }
    }

    Ok(())
}

/// Extract the host from the part of the URL after the scheme: strip any
/// `userinfo@`, take up to the first `/`, `?` or `#`, then drop the `:port`
/// (handling a bracketed `[::1]` IPv6 literal).
fn host_of(after_scheme: &str) -> Option<&str> {
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Strip userinfo (`user:pass@host`); the host is after the last '@'.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return None;
    }
    if let Some(after_bracket) = authority.strip_prefix('[') {
        // `[ipv6]` or `[ipv6]:port` — the host is up to the closing bracket.
        after_bracket.split(']').next()
    } else {
        // `host` or `host:port`.
        authority.split(':').next()
    }
}

/// A case-insensitive `strip_prefix` for the scheme (the common path above is the
/// exact-case fast path; this handles `HTTPS://` etc.).
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Whether an IP literal is a non-routable / SSRF-sensitive target.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped v6 address (`::ffff:a.b.c.d`) is really a v4 target;
            // range-check the embedded v4 so it cannot bypass the v4 rules.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_v4(mapped);
            }
            is_blocked_v6(v6)
        }
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    v4.is_loopback()            // 127.0.0.0/8
        || v4.is_private()      // 10/8, 172.16/12, 192.168/16
        || v4.is_link_local()   // 169.254.0.0/16 — includes the metadata 169.254.169.254
        || v4.is_unspecified()  // 0.0.0.0
        || v4.is_broadcast()    // 255.255.255.255
        || v4.is_documentation()
        || is_shared_v4(v4) // 100.64.0.0/10 CGNAT — not a legitimate webhook target
}

/// 100.64.0.0/10, the RFC 6598 carrier-grade-NAT shared range (`is_shared` is
/// still unstable in std, so it is spelled out here).
fn is_shared_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (o[1] & 0xc0) == 0x40
}

fn is_blocked_v6(v6: Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() {
        return true;
    }
    let seg0 = v6.segments()[0];
    // fe80::/10 unicast link-local (std's `is_unicast_link_local` is unstable).
    let link_local = (seg0 & 0xffc0) == 0xfe80;
    // fc00::/7 unique-local (std's `is_unique_local` is unstable).
    let unique_local = (seg0 & 0xfe00) == 0xfc00;
    link_local || unique_local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_public_https_url_passes() {
        assert_eq!(validate_egress("https://hooks.example.com/ingest"), Ok(()));
        // A public IP literal is fine.
        assert_eq!(validate_egress("https://93.184.216.34/x"), Ok(()));
    }

    #[test]
    fn non_https_is_refused() {
        assert_eq!(
            validate_egress("http://hooks.example.com/"),
            Err(EgressError::NotHttps)
        );
    }

    #[test]
    fn the_cloud_metadata_address_is_blocked() {
        assert_eq!(
            validate_egress("https://169.254.169.254/latest/meta-data/"),
            Err(EgressError::Blocked)
        );
    }

    #[test]
    fn loopback_private_and_localhost_are_blocked() {
        for url in [
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://api.localhost/x",
            "https://10.0.0.5/x",
            "https://192.168.1.10:8443/x",
            "https://172.16.9.9/x",
            "https://[::1]/x",
            "https://[fe80::1]/x",
            "https://[fc00::1]/x",
        ] {
            assert_eq!(validate_egress(url), Err(EgressError::Blocked), "{url}");
        }
    }

    #[test]
    fn an_ipv4_mapped_v6_loopback_cannot_bypass_the_v4_rules() {
        assert_eq!(
            validate_egress("https://[::ffff:127.0.0.1]/x"),
            Err(EgressError::Blocked)
        );
    }

    #[test]
    fn userinfo_and_port_do_not_hide_the_host() {
        assert_eq!(
            validate_egress("https://user:pass@127.0.0.1:9000/x"),
            Err(EgressError::Blocked)
        );
    }
}
