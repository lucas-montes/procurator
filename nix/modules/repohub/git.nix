{config, lib, pkgs, ...}:
with lib; let
  cfg = config.services.procurator.gitServer;
  # TO make it executable
  postReceiveHook = pkgs.writeScript "post-receive" (builtins.replaceStrings
    ["http://localhost:3000/api/builds"]
    [cfg.ciWebhookUrl]
    (builtins.readFile ./post-receive));
  # Create git config file the same way as the hook
  gitConfig = pkgs.writeText "gitconfig" ''
    [core]
        hooksPath = ${cfg.path}/hooks
    [safe]
        directory = *
  '';
in {
  options.services.procurator.gitServer = {
    enable = mkEnableOption "Procurator git server (bare repos, git-shell, post-receive CI hook)";

    path = mkOption {
      type = types.str;
      default = "/var/lib/git-server";
      description = "Root directory holding the bare git repositories.";
    };

    authorizedKeys = mkOption {
      type = types.listOf types.str;
      default = [];
      example = ["ssh-ed25519 AAAA... dev@laptop"];
      description = "SSH public keys allowed to access the repositories via the `git` user.";
    };

    admins = mkOption {
      type = types.listOf types.str;
      default = [];
      example = ["lucas"];
      description = "System users added to the `git` group so they can create/manage bare repos directly.";
    };

    ciWebhookUrl = mkOption {
      type = types.str;
      default = "http://localhost:8082/api/builds";
      description = "URL the post-receive hook notifies when pushes land (Procurator CI API).";
    };
  };

  config = mkIf cfg.enable {
    users.groups.git = {};

    # Why: hook payloads need git metadata files readable; admins manage repos in place.
    users.users = {
      git = {
        isSystemUser = true;
        group = "git";
        home = cfg.path;
        createHome = true;
        shell = "${pkgs.git}/bin/git-shell";
        openssh.authorizedKeys.keys = cfg.authorizedKeys;
      };
    } // listToAttrs (map
      (name: nameValuePair name {extraGroups = ["git"];})
      cfg.admins);

    # Create the post-receive hook in /etc with executable permissions
    environment.etc."git-server/post-receive" = {
      source = postReceiveHook;
      mode = "0755";
    };

    # Create git config in /etc
    environment.etc."git-server/gitconfig" = {
      source = gitConfig;
      mode = "0644";
    };

    # Define filesystem structure and permissions using systemd-tmpfiles
    systemd.tmpfiles.rules = [
      # Create the base git server directory with correct permissions FIRST
      "d ${cfg.path} 2775 git git - -"

      # Create hooks directory with group write permissions
      "d ${cfg.path}/hooks 2775 git git - -"

      # Create symlink from hooks directory to the actual post-receive script
      "L+ ${cfg.path}/hooks/post-receive - - - - /etc/git-server/post-receive"

      # Create symlink to git config file for the git user
      "L+ ${cfg.path}/.gitconfig - - - - /etc/git-server/gitconfig"

      # Z = recursively fix ownership/permissions (cleanup pass)
      "Z ${cfg.path} 2775 git git - -"
    ];
  };
}
