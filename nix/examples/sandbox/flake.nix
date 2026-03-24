{
  description = "Sandbox example — run a Python script in an isolated Cloud Hypervisor VM";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    procurator.url = "path:../..";
  };

  outputs = {
    self,
    nixpkgs,
    procurator,
    ...
  }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs { inherit system; };
    pLib = procurator.lib.${system};

    # ── Build the sandbox ────────────────────────────────────────────
    # This is the Dockerfile-like API:
    #   entrypoint = CMD
    #   localFiles = COPY
    #   packages = RUN apt-get install
    #   allowedDomains = network policy
    sandbox = pLib.mkSandbox {
      # What to run (like CMD in a Dockerfile)
      entrypoint = "python3 /workspace/main.py";

      # Copy local files into the VM (like COPY in a Dockerfile)
      localFiles = {
        "/workspace" = ./workspace;
      };

      # Extra config files as inline strings
      inlineFiles = {
        "/etc/sandbox/config.toml" = ''
          [sandbox]
          name = "python-sandbox-example"
          log_level = "info"
        '';
      };

      # Nix packages available in the VM (like RUN apt-get install)
      packages = p: [ p.python3 p.curl ];

      # VM resources
      cpu = 2;
      memoryMb = 1024;

      # Network: only allow these domains (empty = full access)
      allowedDomains = [ "www.python.org" ];

      # Auto-shutdown when entrypoint finishes
      autoShutdown = true;
    };

  in {
    # ── Packages ─────────────────────────────────────────────────────
    packages.${system} = {
      # The rootfs image
      image = sandbox.image;

      # The VM spec JSON (for the procurator worker)
      vmSpec = sandbox.vmSpecJson;

      # The launch script (run with: nix run .#launch)
      launch = sandbox.launchScript;

      default = sandbox.launchScript;
    };

    # ── Apps ─────────────────────────────────────────────────────────
    apps.${system} = {
      # nix run .#launch — start the sandbox VM
      launch = {
        type = "app";
        program = "${sandbox.launchScript}/bin/launch-sandbox";
      };

      default = {
        type = "app";
        program = "${sandbox.launchScript}/bin/launch-sandbox";
      };

      # nix run .#show-spec — print the VM spec JSON
      show-spec = {
        type = "app";
        program = "${pkgs.writeShellScriptBin "show-spec" ''
          cat ${sandbox.vmSpecJson}
        ''}/bin/show-spec";
      };
    };
  };
}
