import unittest
from captive_check import matches_dhcp_config, matches_dns_answer


class CaptiveCheckTests(unittest.TestCase):
    def test_dhcp_requires_explicit_local_gateway_and_dns(self):
        address = "192.168.16.1"
        config = f"Address={address}\nGateway={address}\nDNSList={address}\n"
        self.assertTrue(matches_dhcp_config(config, address))
        for key in ("Address", "Gateway", "DNSList"):
            self.assertFalse(matches_dhcp_config(config.replace(f"{key}={address}\n", ""), address))
            self.assertFalse(matches_dhcp_config(config.replace(f"{key}={address}", f"{key}=10.0.0.1"), address))

    def test_dns_server_address_is_not_mistaken_for_query_answer(self):
        server = "Server:\t192.168.16.1\nAddress:\t192.168.16.1:53\n"
        self.assertFalse(matches_dns_answer(server, "192.168.16.1"))
        self.assertFalse(matches_dns_answer(server + "Name:\tcaptive.apple.com\nAddress: 10.0.0.1\n", "192.168.16.1"))
        self.assertTrue(matches_dns_answer(server + "Name:\tcaptive.apple.com\nAddress: 192.168.16.1\n", "192.168.16.1"))
