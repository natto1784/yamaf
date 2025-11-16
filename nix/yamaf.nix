{
  craneLib,
  lib,
  src,
  withCleanup ? false,
}:
let
  cargoArtifacts = craneLib.buildDepsOnly { inherit src; };
in
craneLib.buildPackage {
  inherit cargoArtifacts src;
  doCheck = false;

  cargoExtraArgs = lib.optionalString withCleanup "--features cleanup";
}
