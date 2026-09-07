{
  description = "Minimal stack fixture for smoke testing pcr stack up";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
  };

  outputs = { self, nixpkgs }: {
    stack = {
      services = {
        svc-a = {
          cmd = [ "echo" "hello from svc-a" ];
          oneShot = true;
        };
        svc-b = {
          cmd = [ "echo" "hello from svc-b" ];
          dependsOn = [ "svc-a" ];
          oneShot = true;
        };
      };
    };
  };
}
