#!/bin/sh
set -e

REPO="majiayu000/remem"
INSTALL_DIR="${REMEM_INSTALL_DIR:-${INSTALL_DIR:-${HOME}/.local/bin}}"
REMEM_NO_CONFIG="${REMEM_NO_CONFIG:-0}"

# Detect OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${OS}" in
  linux)  OS_NAME="linux" ;;
  darwin) OS_NAME="darwin" ;;
  *)      echo "Unsupported OS: ${OS}"; exit 1 ;;
esac

case "${ARCH}" in
  x86_64|amd64)  ARCH_NAME="x64" ;;
  aarch64|arm64) ARCH_NAME="arm64" ;;
  *)             echo "Unsupported architecture: ${ARCH}"; exit 1 ;;
esac

BINARY_NAME="remem-${OS_NAME}-${ARCH_NAME}"

# Get latest release tag
if [ -z "${REMEM_VERSION}" ]; then
  REMEM_VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')"
fi

if [ -z "${REMEM_VERSION}" ]; then
  echo "Failed to detect latest version. Set REMEM_VERSION manually."
  exit 1
fi

URL="https://github.com/${REPO}/releases/download/${REMEM_VERSION}/${BINARY_NAME}.tar.gz"
CHECKSUM_URL="https://github.com/${REPO}/releases/download/${REMEM_VERSION}/SHA256SUMS"

echo "Installing remem ${REMEM_VERSION} (${OS_NAME}/${ARCH_NAME})..."

# Create install directory
mkdir -p "${INSTALL_DIR}"

# Download and extract
TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

curl -fsSL "${URL}" -o "${TMPDIR}/remem.tar.gz"
if ! curl -fsSL "${CHECKSUM_URL}" -o "${TMPDIR}/SHA256SUMS"; then
  echo "Failed to download SHA256SUMS from ${CHECKSUM_URL}"
  exit 1
fi

CHECKSUM_ENTRY="${BINARY_NAME}.tar.gz"
if ! EXPECTED_CHECKSUM="$(awk -v file="${CHECKSUM_ENTRY}" '
  {
    path = $2
    sub(/^\*/, "", path)
    n = split(path, parts, "/")
    basename = parts[n]
  }
  basename == file {
    print $1
    found = 1
    exit
  }
  END {
    if (!found) {
      exit 1
    }
  }
' "${TMPDIR}/SHA256SUMS")"; then
  echo "Missing checksum entry for ${CHECKSUM_ENTRY} in SHA256SUMS"
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_CHECKSUM="$(sha256sum "${TMPDIR}/remem.tar.gz" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  ACTUAL_CHECKSUM="$(shasum -a 256 "${TMPDIR}/remem.tar.gz" | awk '{print $1}')"
else
  echo "Cannot verify checksum: neither sha256sum nor shasum is available."
  exit 1
fi

if [ "${ACTUAL_CHECKSUM}" != "${EXPECTED_CHECKSUM}" ]; then
  echo "Checksum verification failed for ${CHECKSUM_ENTRY}"
  echo "Expected: ${EXPECTED_CHECKSUM}"
  echo "Actual:   ${ACTUAL_CHECKSUM}"
  exit 1
fi

echo "Verified checksum for ${CHECKSUM_ENTRY}"
tar xzf "${TMPDIR}/remem.tar.gz" -C "${TMPDIR}"
mv "${TMPDIR}/remem" "${INSTALL_DIR}/remem"
chmod +x "${INSTALL_DIR}/remem"
# macOS ARM requires ad-hoc codesign after replacing binary
if [ "$(uname -s)" = "Darwin" ] && [ "$(uname -m)" = "arm64" ]; then
  if ! command -v codesign >/dev/null 2>&1; then
    echo "codesign is required on macOS ARM after replacing ${INSTALL_DIR}/remem"
    exit 1
  fi
  if ! codesign -s - -f "${INSTALL_DIR}/remem"; then
    echo "Failed to ad-hoc codesign ${INSTALL_DIR}/remem"
    exit 1
  fi
fi

"${INSTALL_DIR}/remem" --version >/dev/null

echo "Installed to ${INSTALL_DIR}/remem"

# Check PATH
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo ""
    echo "WARNING: ${INSTALL_DIR} is not in your PATH."
    echo "Add this to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\${PATH}\""
    ;;
esac

if [ "${REMEM_NO_CONFIG}" = "1" ]; then
  echo ""
  echo "Skipped hook/MCP configuration because REMEM_NO_CONFIG=1."
  echo "Run 'remem install' later to configure Claude Code/Codex."
  exit 0
fi

# Run install to configure hooks + MCP
echo ""
echo "Configuring Claude Code/Codex hooks and MCP..."
REMEM_INSTALL_BINARY="${INSTALL_DIR}/remem" "${INSTALL_DIR}/remem" install

echo ""
echo "Done! Restart Claude Code or Codex to activate remem."
echo "Run 'remem status' to verify installation."
