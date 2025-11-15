{
  lib,
  rustPlatform,
  pkg-config,
  stdenv,
  self ? null,
}:

rustPlatform.buildRustPackage rec {
  pname = "local-logger";
  version = "0.1.0";

  # Use flake self if available, otherwise assume we're in the source directory
  src = if self != null then self else ./..;

  cargoLock = {
    lockFile = src + "/Cargo.lock";
  };

  nativeBuildInputs = [ pkg-config ];

  checkPhase = ''
    runHook preCheck
    cargo test --bins
    runHook postCheck
  '';

  postInstall = ''
    if [ -f "$out/bin/local-logger" ]; then
      echo "Binary installed successfully"
    else
      echo "Warning: Binary not found at expected location"
    fi
  '';

  meta = with lib; {
    description = "Local Logger - MCP server, hook logger, and HTTPS proxy";
    longDescription = ''
      A multi-purpose tool that serves as:
      1. MCP (Model Context Protocol) server for logging operations
      2. Claude Code hook processor for logging tool usage events
      3. HTTPS MITM proxy for recording Claude API traffic

      All logs are stored in unified NDJSON format with daily rotation.
    '';
    homepage = "https://github.com/datawizz/local-logger";
    license = licenses.mit;
    maintainers = [ ];
    mainProgram = "local-logger";
    platforms = platforms.unix;
  };
}
