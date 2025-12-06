{
  description = "Minimal test flake for CI parser testing";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.11";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      packages.${system} = {
        # Simple package that just creates a file
        default = pkgs.writeTextFile {
          name = "test-app";
          text = ''
            #!/bin/sh
            echo "Hello from test app!"
          '';
          executable = true;
        };

        # Another simple package for testing multiple packages
        helper = pkgs.writeTextFile {
          name = "test-helper";
          text = ''
            #!/bin/sh
            echo "Helper utility"
          '';
          executable = true;
        };
      };

      checks.${system} = {
        # Fast-running test that always passes
        basic-test = pkgs.runCommand "basic-test" {} ''
          echo "Running basic test..."
          sleep 1
          echo "Test passed!" > $out
        '';

        # Another quick test
        format-check = pkgs.runCommand "format-check" {} ''
          echo "Checking formatting..."
          echo "Format OK" > $out
        '';

        # Test with a small delay to see timing
        slow-test = pkgs.runCommand "slow-test" {} ''
          echo "Running slow test..."
          sleep 2
          echo "Slow test completed" > $out
        '';
      };

      apps.${system}.default = {
        type = "app";
        program = "${self.packages.${system}.default}/bin/test-app";
      };

      devShells.${system}.default = pkgs.mkShell {
        buildInputs = with pkgs; [ hello ];
      };
    };
}
