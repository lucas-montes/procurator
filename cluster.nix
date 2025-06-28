{
  cluster = {
    apps = {
      dummy = {
        production = {
          cpu = 2.2;
          memory = 2;
          memory_unit = "GB";
          area = "production";
          app = (import ./default.nix).outPath;
        };
        staging = {
          cpu = 1.1;
          memory = 1;
          memory_unit = "GB";
          area = "staging";
          app = (import ./default.nix).outPath;
        };
      };
    };
  };
}
