{
  config,
  ...
}:
{
  rustEnv.managedCargo = {
    enable = true;
  };

  rustEnv.package.manifestPath = ./crates/syntax-tree/Cargo.toml;

  scripts = {
    show-workspace-manifest.exec = ''
      cat ${config.outputs.cargo_manifest}
    '';

    workspace-check.exec = ''
      cargo check --workspace
    '';
  };

  enterShell = ''
    echo "Run: show-workspace-manifest"
    echo "Run: workspace-check"
  '';

  enterTest = ''
    set -euo pipefail

    cargo --version
    grep -F 'arc-swap = "1.8.2"' ${config.outputs.cargo_manifest}
    grep -F 'regex-cursor = "0.1.5"' ${config.outputs.cargo_manifest}
    grep -F 'ropey = "1.6.1"' ${config.outputs.cargo_manifest}
    grep -F 'slab = "0.4.12"' ${config.outputs.cargo_manifest}
    grep -F 'version = "0.2.3"' ${config.outputs.cargo_manifest}
    cargo metadata --no-deps >/dev/null
    cargo check --workspace --all-targets --all-features
    cargo test --workspace --all-targets --all-features
  '';
}
