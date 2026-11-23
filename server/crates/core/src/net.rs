//! One answer to "may the server connect to this address?", for the outgoing requests whose target a
//! user chooses (a push subscription's endpoint, an OAuth client's metadata document).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Public means a routable unicast address: not loopback, private, link-local (which holds the cloud
/// metadata address), unique-local, site-local, carrier-grade NAT, documentation, benchmarking,
/// multicast or unspecified. An IPv6 address that carries an IPv4 one (mapped `::ffff:a.b.c.d`,
/// compatible `::a.b.c.d`, NAT64 `64:ff9b::/96`, 6to4 `2002::/16`) is judged by the IPv4 address it
/// carries; Teredo (`2001::/32`) and the other IETF protocol assignments (`2001::/23`) are refused.
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    let seg = ip.segments();
    let embedded = |high: u16, low: u16| {
        let [a, b] = high.to_be_bytes();
        let [c, d] = low.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    // IPv4-compatible (::a.b.c.d, deprecated): everything below ::/96 but the unspecified and loopback
    // addresses (which the checks below refuse) carries an IPv4 address.
    if seg[..6].iter().all(|part| *part == 0) {
        return false;
    }
    // NAT64 well-known prefix (RFC 6052): the last 32 bits are the IPv4 target. The local-use prefix
    // 64:ff9b:1::/48 (RFC 8215) is internal by definition.
    if seg[0] == 0x0064 && seg[1] == 0xff9b {
        return seg[2..6].iter().all(|part| *part == 0) && is_public_v4(embedded(seg[6], seg[7]));
    }
    // 6to4 (RFC 3056): the IPv4 address follows the prefix.
    if seg[0] == 0x2002 {
        return is_public_v4(embedded(seg[1], seg[2]));
    }
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || seg[0] & 0xfe00 == 0xfc00 // unique local
        || seg[0] & 0xffc0 == 0xfe80 // link local
        || seg[0] & 0xffc0 == 0xfec0 // site local (deprecated)
        || (seg[0] == 0x2001 && seg[1] < 0x0200) // IETF protocol assignments, Teredo, benchmarking
        || (seg[0] == 0x2001 && seg[1] == 0x0db8) // documentation
        || (seg[0] == 0x3fff && seg[1] < 0x1000) // documentation (RFC 9637)
        || (seg[0] == 0x0100 && seg[1..4].iter().all(|part| *part == 0))) // discard-only
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        || (a == 100 && (64..128).contains(&b)) // carrier-grade NAT
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 88 && c == 99) // 6to4 relay anycast (deprecated)
        || (a == 198 && (18..20).contains(&b)) // benchmarking
        || a >= 240)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_special_and_embedding_addresses_are_refused() {
        for blocked in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "198.18.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "fec0::1", // site local
            "ff02::1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::10.0.0.1", // IPv4-compatible
            "::8.8.8.8",
            "64:ff9b::a00:1",     // NAT64 to 10.0.0.1
            "64:ff9b::7f00:1",    // NAT64 to 127.0.0.1
            "64:ff9b::a9fe:a9fe", // NAT64 to the metadata address
            "64:ff9b:1::1",       // NAT64 local use
            "2002:a00:1::1",      // 6to4 of 10.0.0.1
            "2002:7f00:1::",      // 6to4 of 127.0.0.1
            "2002:a9fe:a9fe::1",
            "2001::1",   // Teredo
            "2001:2::1", // benchmarking
            "2001:db8::1",
            "100::1",
        ] {
            assert!(!is_public_address(blocked.parse().unwrap()), "{blocked}");
        }
    }

    #[test]
    fn routable_addresses_are_allowed() {
        for open in [
            "8.8.8.8",
            "1.1.1.1",
            "142.250.1.1",
            "2606:4700:4700::1111",
            "2607:f8b0:4004::1",
            "64:ff9b::808:808", // NAT64 to 8.8.8.8
            "2002:808:808::1",  // 6to4 of 8.8.8.8
            "2a00:1450:4001::1",
        ] {
            assert!(is_public_address(open.parse().unwrap()), "{open}");
        }
    }
}
