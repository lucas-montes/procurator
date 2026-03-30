{
  # Aggregator module for procurator host pieces.
  # Imports the focused modules so callers can enable vmm and/or worker.
  imports = [./vmm.nix ./service.nix];
}
