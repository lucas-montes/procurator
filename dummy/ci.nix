{ pkgs ? import <nixpkgs> { } }:

{
  # CI-specific checks that run in the CI environment
  # These are separate from regular development checks for better control

  checks = {
    # Basic format/lint checks (fast)
    formatting = pkgs.runCommand "check-formatting" { } ''
      echo "Checking code formatting..."
      # Add your formatting checks here
      # e.g., nixpkgs-fmt --check ${./.}
      echo "Formatting OK" > $out
    '';

    # Run the test script
    test-script = pkgs.runCommand "run-tests" {
      buildInputs = [ pkgs.bash ];
    } ''
      echo "Running test script..."
      cp ${./test_dummy.sh} test_dummy.sh
      chmod +x test_dummy.sh
      bash test_dummy.sh
      echo "Tests passed" > $out
    '';

    # Build verification (ensures the project builds)
    build-check = pkgs.runCommand "verify-build" {
      buildInputs = [ pkgs.gcc ];
    } ''
      echo "Verifying project can build..."
      cp ${./main.c} main.c
      ${pkgs.gcc}/bin/gcc -o dummy main.c
      ./dummy
      echo "Build verification passed" > $out
    '';

    # Security checks (placeholder for future)
    security = pkgs.runCommand "security-checks" { } ''
      echo "Running security checks..."
      # Add security scanning here
      # e.g., git-secrets, trivy, etc.
      echo "Security checks passed" > $out
    '';

    # License compliance (placeholder)
    licenses = pkgs.runCommand "license-check" { } ''
      echo "Checking license compliance..."
      # Add license checking here
      echo "License compliance OK" > $out
    '';
  };

  # CI-specific build configuration
  # These settings optimize for CI environment
  build = {
    # Enable parallel building in CI
    enableParallelBuilding = true;

    # Stricter error checking in CI
    strictDeps = true;

    # Generate build reports
    separateDebugInfo = false;

    # Optimize for build cache reuse
    preferLocalBuild = false;
  };

  # CI environment metadata
  meta = {
    description = "CI checks for dummy project";
    # Specify which platforms to test on
    platforms = [ "x86_64-linux" "aarch64-linux" ];
    # Maintainers for CI notifications
    maintainers = [ "ci-team" ];
  };
}
