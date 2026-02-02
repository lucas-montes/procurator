{
  description = "Auto-generated flake for api, web";
  inputs = {
    nixpkgs = "github:NixOS/nixpkgs/nixos-unstable";
  };
  outputs = {
    packages = {
      procurator-web = {
        name = "procurator-web";
        toolchain = {
          language = "JavaScript";
          package_manager = "Npm";
          version = { unknown = NULL; };
        };
        dependencies = [
        ];
        metadata = {
          version = "latest";
          authors = [
          ];
        };
      };
      procurator-api = {
        name = "procurator-api";
        toolchain = {
          language = "JavaScript";
          package_manager = "Npm";
          version = { unknown = NULL; };
        };
        dependencies = [
        ];
        metadata = {
          version = "latest";
          authors = [
          ];
        };
      };
    };
    dev_shells = {
      toolchains = [
        {
          language = "JavaScript";
          package_manager = "Npm";
          version = { unknown = NULL; };
        }
      ];
      dependencies = [
      ];
      env = {
      };
      services = [
      ];
    };
    checks = {
    };
    procurator = {
      services = [
      ];
      project = {
        name = "api";
        languages = [
          "JavaScript"
        ];
        package_managers = [
          "Npm"
        ];
      };
    };
  };
}