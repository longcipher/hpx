use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub fn is_forbidden_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_forbidden_v4(v4),
        IpAddr::V6(v6) => is_forbidden_v6(v6),
    }
}

/// Check if a host (IP literal or hostname) resolves to a forbidden address.
///
/// For IP literals, checks directly. For hostnames, resolves via DNS first
/// and checks all resolved addresses — this prevents DNS rebinding attacks
/// where a hostname resolves to a private/special-use IP.
///
/// Returns `true` if any resolved address is forbidden, `false` otherwise.
/// DNS resolution failures are treated as forbidden (fail-closed) since we
/// cannot verify the IP is safe.
pub async fn is_forbidden_resolved(host: &str) -> bool {
    // Strip brackets for IPv6 literals: "[::1]" → "::1"
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = stripped.parse::<IpAddr>() {
        return is_forbidden_ip(&ip);
    }

    // Hostname — resolve via DNS and check all resulting IPs.
    let mut addrs = match tokio::net::lookup_host((stripped, 0)).await {
        Ok(addrs) => addrs,
        Err(_) => return true, // fail-closed: can't verify IP safety
    };

    addrs.any(|addr| is_forbidden_ip(&addr.ip()))
}

/// Check if an IP-literal host is forbidden.
///
/// Only checks direct IP address literals (e.g. `127.0.0.1`, `::1`).
/// For hostnames, use [`is_forbidden_resolved`] which resolves DNS first.
pub fn is_forbidden_host(host: &str) -> bool {
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = stripped.parse::<IpAddr>() {
        return is_forbidden_ip(&ip);
    }
    false
}

fn is_forbidden_v4(ip: &Ipv4Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    let o = ip.octets();
    // RFC1918
    if o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168) {
        return true;
    }
    // Link-local
    if o[0] == 169 && o[1] == 254 {
        return true;
    }
    // Broadcast
    if *ip == Ipv4Addr::BROADCAST {
        return true;
    }
    // Documentation
    if (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
    {
        return true;
    }
    false
}

fn is_forbidden_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    let s = ip.segments();
    // Link-local fe80::/10
    if (s[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // Unique-local fd00::/8
    if (s[0] & 0xff00) == 0xfd00 {
        return true;
    }
    // Documentation 2001:db8::/32
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return true;
    }
    // IPv4-mapped ::ffff:0:0/96 — check the mapped v4 part
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff {
        let mapped = Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        );
        return is_forbidden_v4(&mapped);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_v4() {
        assert!(is_forbidden_host("127.0.0.1"));
        assert!(is_forbidden_host("127.255.255.255"));
        assert!(is_forbidden_ip(&"127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn loopback_v6() {
        assert!(is_forbidden_host("::1"));
        assert!(is_forbidden_host("[::1]"));
    }

    #[test]
    fn unspecified() {
        assert!(is_forbidden_host("0.0.0.0"));
        assert!(is_forbidden_host("::"));
    }

    #[test]
    fn rfc1918() {
        assert!(is_forbidden_host("10.0.0.1"));
        assert!(is_forbidden_host("10.255.255.255"));
        assert!(is_forbidden_host("172.16.0.1"));
        assert!(is_forbidden_host("172.31.255.255"));
        assert!(is_forbidden_host("192.168.1.1"));
        assert!(is_forbidden_host("192.168.255.255"));
        // boundaries outside RFC1918
        assert!(!is_forbidden_host("11.0.0.1"));
        assert!(!is_forbidden_host("172.32.0.1"));
        assert!(!is_forbidden_host("192.169.0.1"));
    }

    #[test]
    fn link_local_v4() {
        assert!(is_forbidden_host("169.254.1.1"));
        assert!(is_forbidden_host("169.254.255.255"));
        assert!(!is_forbidden_host("169.253.255.255"));
    }

    #[test]
    fn link_local_v6() {
        assert!(is_forbidden_host("fe80::1"));
        assert!(is_forbidden_ip(&"fe80::abcd:1234".parse().unwrap()));
    }

    #[test]
    fn broadcast() {
        assert!(is_forbidden_host("255.255.255.255"));
    }

    #[test]
    fn documentation_ranges() {
        assert!(is_forbidden_host("192.0.2.1"));
        assert!(is_forbidden_host("198.51.100.1"));
        assert!(is_forbidden_host("203.0.113.1"));
        assert!(is_forbidden_host("2001:db8::1"));
    }

    #[test]
    fn unique_local_v6() {
        assert!(is_forbidden_host("fd00::1"));
        assert!(is_forbidden_ip(&"fd12:3456:789a::1".parse().unwrap()));
        assert!(!is_forbidden_host("fc00::1"));
    }

    #[test]
    fn ipv4_mapped_v6() {
        assert!(is_forbidden_host("::ffff:127.0.0.1"));
        assert!(is_forbidden_host("::ffff:10.0.0.1"));
        assert!(is_forbidden_ip(&"::ffff:192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn safe_addresses_pass() {
        assert!(!is_forbidden_host("93.184.216.34"));
        assert!(!is_forbidden_host("2606:4700::1"));
        assert!(!is_forbidden_host("8.8.8.8"));
        assert!(!is_forbidden_host("example.com"));
    }

    #[test]
    fn multicast_forbidden() {
        assert!(is_forbidden_host("224.0.0.1"));
        assert!(is_forbidden_host("ff02::1"));
    }

    // --- Async DNS resolution tests ---

    #[tokio::test]
    async fn resolved_blocks_loopback_ip_literal() {
        assert!(is_forbidden_resolved("127.0.0.1").await);
        assert!(is_forbidden_resolved("::1").await);
    }

    #[tokio::test]
    async fn resolved_blocks_localhost_hostname() {
        // "localhost" resolves to 127.0.0.1 / ::1 on most systems.
        assert!(is_forbidden_resolved("localhost").await);
    }

    #[tokio::test]
    async fn resolved_blocks_private_hostname() {
        // "localtest" with /etc/hosts entry pointing to 10.0.0.1 would be blocked.
        // For portability, test with an IP literal that's already forbidden.
        assert!(is_forbidden_resolved("10.0.0.1").await);
        assert!(is_forbidden_resolved("192.168.1.1").await);
    }

    #[tokio::test]
    async fn resolved_allows_public_hostname() {
        // example.com resolves to public IPs — must NOT be forbidden.
        assert!(!is_forbidden_resolved("example.com").await);
    }

    #[tokio::test]
    async fn resolved_fail_closed_on_unresolvable() {
        // A non-existent domain should fail DNS resolution → fail-closed → forbidden.
        assert!(is_forbidden_resolved("this-domain-does-not-exist-abc123.test").await);
    }
}
