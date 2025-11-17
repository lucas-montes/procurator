{
  description = "Procurator - Automatic cache upload for Nix CI";

  outputs = { self }: {
    # Library functions for use in project flakes
    lib = {
      # Helper to generate upload script
      mkUploadScript = { ciServer, ciUser ? "git", verbose ? false }: ''
        #!/bin/sh
        set -eu
        set -f

        ${if verbose then ''
          echo "📦 Uploading to Procurator @ ${ciServer}..." >&2
        '' else ""}

        {
          nix copy --to ssh-ng://${ciUser}@${ciServer} $OUT_PATHS 2>&1 || true
          ${if verbose then ''
            echo "✓ Cached to CI" >&2
          '' else ""}
        } &

        exit 0
      '';

      # Helper to create nixConfig
      mkNixConfig = { ciServer, ciUser ? "git", verbose ? false }: {
        post-build-hook = self.lib.mkUploadScript { inherit ciServer ciUser verbose; };
        extra-substituters = [ "ssh-ng://${ciUser}@${ciServer}" ];
        connect-timeout = 5;
      };

      # Simple helper: just pass the CI server
      withProcurator = ciServer: self.lib.mkNixConfig { inherit ciServer; };

      # Full version with all options
      withProcuratorFull = self.lib.mkNixConfig;
    };

}
