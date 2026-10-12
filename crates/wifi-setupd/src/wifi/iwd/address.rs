use super::Error;

/// Injected separately from D-Bus so private-bus tests never inspect host links.
pub trait AddressSource: Send + Sync {
    fn has_usable_address(&self, interface: &str) -> Result<bool, Error>;
}

/// The development image uses IPv4 DHCP. IPv6-only readiness is deliberately
/// unqualified here: getifaddrs cannot distinguish tentative IPv6 addresses.
pub struct InterfaceAddresses;
impl AddressSource for InterfaceAddresses {
    fn has_usable_address(&self, interface: &str) -> Result<bool, Error> {
        let addresses = nix::ifaddrs::getifaddrs().map_err(|_| Error::AddressUnavailable)?;
        Ok(addresses
            .filter(|entry| entry.interface_name == interface)
            .any(|entry| {
                entry
                    .address
                    .and_then(|addr| addr.as_sockaddr_in().map(|addr| addr.ip()))
                    .is_some_and(usable_ipv4)
            }))
    }
}

fn usable_ipv4(address: std::net::Ipv4Addr) -> bool {
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_link_local()
        && !address.is_multicast()
        && !address.is_broadcast()
        && address.octets()[0] != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_excludes_unusable_addresses_but_does_not_require_internet() {
        for address in [
            "0.0.0.0",
            "127.0.0.1",
            "169.254.1.2",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!usable_ipv4(address.parse().unwrap()));
        }
        assert!(usable_ipv4("192.168.1.2".parse().unwrap()));
    }
}
