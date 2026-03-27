{}:{

  # We set up the virtual machine manager (VMM) to run the virtual machines for the procurator service.
  # This is used to run the virtual machines that are used to run the various services that are used by the procurator service.

  # br0 is the host-side bridge used only for VMs. Because the host NIC is Wi‑Fi
  # we do NOT attach the physical wireless interface to this bridge (Wi‑Fi bridging
  # to an AP usually doesn't work). The bridge just provides a L2 network for VMs.
  networking.interfaces.br0.ipv4.addresses = [ "192.168.100.1/24" ];

  # Enable NAT/masquerading so VMs on br0 can reach the outside via the Wi‑Fi device.
  # - enable: turns on NAT
  # - internalInterfaces: interfaces that will be NATed (the VMs' bridge)
  # - externalInterface: the real uplink (your Wi‑Fi device)
  networking.nat = {
    enable = true;
    internalInterfaces = [ "br0" ];
    externalInterface = "wlp98s0";
  };

  # dnsmasq provides DHCP (and optionally DNS) to dynamically created VMs on br0.
  # - enable: start dnsmasq
  # - interfaces: which interface(s) dnsmasq listens on (the VMs' bridge)
  # - dhcpRange: the DHCP pool, lease time (here 12h)
  # dnsmasq is simple and suited for VMs that come and go dynamically.
  services.dnsmasq = {
    enable = true;
    interfaces = [ "br0" ];
    dhcpRange = "192.168.100.10,192.168.100.100,12h";
  };
}
