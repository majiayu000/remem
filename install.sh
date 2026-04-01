#!/bin/sh
set -e

REPO="majiayu000/remem"
INSTALL_DIR="${HOME}/.local/bin"

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

echo "Installing remem ${REMEM_VERSION} (${OS_NAME}/${ARCH_NAME})..."

# Create install directory
mkdir -p "${INSTALL_DIR}"

# Download and extract
TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR}"' EXIT

curl -fsSL "${URL}" -o "${TMPDIR}/remem.tar.gz"
tar xzf "${TMPDIR}/remem.tar.gz" -C "${TMPDIR}"
mv "${TMPDIR}/remem" "${INSTALL_DIR}/remem"
chmod +x "${INSTALL_DIR}/remem"
# macOS ARM requires ad-hoc codesign after replacing binary
if [ "$(uname -s)" = "Darwin" ] && [ "$(uname -m)" = "arm64" ]; then
  codesign -s - -f "${INSTALL_DIR}/remem" 2>/dev/null || true
fi

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

# Run install to configure hooks + MCP
echo ""
echo "Configuring Claude Code hooks and MCP..."
"${INSTALL_DIR}/remem" install

echo ""
echo "Done! Restart Claude Code to activate remem."
echo "Run 'remem status' to verify installation."
