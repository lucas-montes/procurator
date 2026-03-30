{pkgs, ...}: let
  gitServerPath = "/var/lib/git-server";
  # TO make it executable
  postReceiveHook = pkgs.writeScript "post-receive" (builtins.readFile ./post-receive);
  # Create git config file the same way as the hook
  gitConfig = pkgs.writeText "gitconfig" ''
    [core]
        hooksPath = ${gitServerPath}/hooks
    [safe]
        directory = *
  '';
in {
  users.groups.git = {};

  users.users.git = {
    isSystemUser = true;
    group = "git";
    home = gitServerPath;
    createHome = true;
    shell = "${pkgs.git}/bin/git-shell";
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJk9K6n6KDOI9dKTu9ocqKnBF29KVlOlIm413Ci4M8dU lucas@luctop"
    ];
  };

  users.users.lucas.extraGroups = ["git"];

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
    "d ${gitServerPath} 2775 git git - -"

    # Create hooks directory with group write permissions
    "d ${gitServerPath}/hooks 2775 git git - -"

    # Create symlink from hooks directory to the actual post-receive script
    "L+ ${gitServerPath}/hooks/post-receive - - - - /etc/git-server/post-receive"

    # Create symlink to git config file for the git user
    "L+ ${gitServerPath}/.gitconfig - - - - /etc/git-server/gitconfig"

    # Z = recursively fix ownership/permissions (cleanup pass)
    "Z ${gitServerPath} 2775 git git - -"
  ];
}
