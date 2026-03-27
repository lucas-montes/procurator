# makeInitrd — builds a minimal initrd for Cloud Hypervisor VMs.
#
# The initrd provides:
#   - bash + busybox
#   - DHCP networking on all available interfaces (10s timeout)
#   - OpenSSH daemon (key-based only, password auth disabled)
#   - A dedicated non-root user with the supplied SSH public key
#   - Optional extra files copied in at build time
#
# Arguments:
#   pkgs       — nixpkgs package set (injected by flake.nix)
#   lib        — nixpkgs lib (injected by flake.nix)
#
# Returns a function:
#   makeInitrd {
#     sshKey   ? ""          # SSH public key string (optional but recommended)
#     username ? "vm-user"   # dedicated user created inside the initrd
#     files    ? []          # [{ src = path; dst = "/absolute/path"; }]
#   }
#   -> derivation producing an initrd cpio.gz

{ pkgs, lib }:

{
  sshKey   ? "",
  username ? "vm-user",
  files    ? [],
}:

let
  # ── Packages available inside the initrd ────────────────────────────────
  busybox  = pkgs.busybox;
  bash     = pkgs.bash;
  openssh  = pkgs.openssh;
  dhcpcd   = pkgs.dhcpcd;
  coreutils = pkgs.coreutils;

  # ── SSH host keys (generated at build time for reproducibility) ──────────
  # In production you'd want to inject host keys at runtime; for now we
  # generate a fresh ed25519 key pair during the Nix build.
  sshHostKey = pkgs.runCommand "ssh-host-key" { buildInputs = [ openssh ]; } ''
    mkdir -p $out
    ssh-keygen -t ed25519 -N "" -f $out/ssh_host_ed25519_key
  '';

  # ── User authorised_keys file ─────────────────────────────────────────────
  authorizedKeysFile =
    if sshKey != ""
    then pkgs.writeText "authorized_keys" sshKey
    else null;

  # ── /etc/passwd and /etc/shadow (minimal) ────────────────────────────────
  etcPasswd = pkgs.writeText "passwd" ''
    root:x:0:0:root:/root:/bin/sh
    sshd:x:74:74:SSH daemon:/var/empty:/bin/false
    ${username}:x:1000:1000:VM User:/home/${username}:/bin/bash
  '';

  etcShadow = pkgs.writeText "shadow" ''
    root:!:1::::::
    sshd:!:1::::::
    ${username}:!:1::::::
  '';

  etcGroup = pkgs.writeText "group" ''
    root:x:0:
    sshd:x:74:
    ${username}:x:1000:
  '';

  # ── sshd_config ───────────────────────────────────────────────────────────
  sshdConfig = pkgs.writeText "sshd_config" ''
    Port 22
    AddressFamily any
    ListenAddress 0.0.0.0
    ListenAddress ::

    HostKey /etc/ssh/ssh_host_ed25519_key

    PermitRootLogin no
    PasswordAuthentication no
    ChallengeResponseAuthentication no
    UsePAM no

    AllowUsers ${username}

    Subsystem sftp ${openssh}/libexec/sftp-server
  '';

  # ── dhcpcd.conf ───────────────────────────────────────────────────────────
  dhcpcdConf = pkgs.writeText "dhcpcd.conf" ''
    # Try DHCP on all interfaces; give up after 10 seconds per interface
    timeout 10
    noipv6
  '';

  # ── init script ───────────────────────────────────────────────────────────
  initScript = pkgs.writeScript "init" ''
    #!${bash}/bin/bash
    set -e

    export PATH="${busybox}/bin:${bash}/bin:${coreutils}/bin:${openssh}/bin:${dhcpcd}/bin"

    # ── Mount essential filesystems ────────────────────────────────────────
    mount -t proc     none /proc
    mount -t sysfs    none /sys
    mount -t devtmpfs none /dev  2>/dev/null || mount -t tmpfs none /dev
    mkdir -p /dev/pts
    mount -t devpts   none /dev/pts

    echo "cloud-hypervisor initrd booting..."

    # ── Bring up loopback ──────────────────────────────────────────────────
    ip link set lo up 2>/dev/null || true

    # ── DHCP on all ethernet interfaces (10s timeout each) ────────────────
    for iface in $(ls /sys/class/net | grep -v lo); do
      echo "Running DHCP on $iface..."
      ip link set "$iface" up 2>/dev/null || true
      dhcpcd --config /etc/dhcpcd.conf --nobackground --timeout 10 "$iface" \
        2>/dev/null && echo "DHCP acquired on $iface" || echo "DHCP failed on $iface, continuing..."
    done

    # ── Start SSH daemon ───────────────────────────────────────────────────
    mkdir -p /var/empty /run/sshd
    echo "Starting sshd..."
    ${openssh}/bin/sshd -f /etc/ssh/sshd_config -E /dev/console || \
      echo "WARNING: sshd failed to start"

    echo ""
    echo "initrd ready. Dropping to bash shell."
    echo "SSH is available if network is up."
    echo ""

    # ── Drop to interactive shell ──────────────────────────────────────────
    exec ${bash}/bin/bash --login
  '';



in
  # ── Assemble the initrd ───────────────────────────────────────────────────
  pkgs.makeInitrdNG {
    compressor = "gzip";

    contents =
      # Core filesystem skeleton
      [
        { source = "${busybox}/bin/busybox"; target = "/bin/busybox"; }
        { source = "${bash}/bin/bash";       target = "/bin/bash"; }
        { source = "${bash}/bin/bash";       target = "/bin/sh"; }

        # /init
        { source = initScript; target = "/init"; }

        # /etc essentials
        { source = etcPasswd;  target = "/etc/passwd"; }
        { source = etcShadow;  target = "/etc/shadow"; }
        { source = etcGroup;   target = "/etc/group"; }

        # SSH configuration
        { source = sshdConfig;                               target = "/etc/ssh/sshd_config"; }
        { source = "${sshHostKey}/ssh_host_ed25519_key";     target = "/etc/ssh/ssh_host_ed25519_key"; }
        { source = "${sshHostKey}/ssh_host_ed25519_key.pub"; target = "/etc/ssh/ssh_host_ed25519_key.pub"; }

        # DHCP configuration
        { source = dhcpcdConf; target = "/etc/dhcpcd.conf"; }
      ]
      # Authorized keys (only when sshKey is provided)
      ++ lib.optional (authorizedKeysFile != null) {
        source = authorizedKeysFile;
        target = "/home/${username}/.ssh/authorized_keys";
      }
      # Extra user files — each { src, dst } becomes its own source/target entry
      ++ map ({ src, dst }: { source = src; target = dst; }) files;
  }
