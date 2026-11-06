//! Conservative public-address policy shared by media negotiation and the
//! decoder's actual socket/relay checks. No DNS resolution occurs here.
use std::net::IpAddr;

pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || a == 0
                || a >= 240
                || a == 100 && (64..=127).contains(&b)
                || a == 192 && b == 0 && (c == 0 || c == 2)
                || a == 192 && b == 88 && c == 99
                || a == 198 && (b == 18 || b == 19 || b == 51 && c == 100)
                || a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public_ip(mapped.into());
            }
            let s = ip.segments();
            // Global unicast only, excluding 6to4, documentation, and the
            // IETF special-purpose /23 (including benchmark/ORCHID/Teredo).
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0xdb8))
        }
    }
}
