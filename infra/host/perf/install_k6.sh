#!/usr/bin/env bash
set -euo pipefail

VERSION="${K6_VERSION:-1.6.1}"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}-${ARCH}" in
  Darwin-arm64)
    ARCHIVE="k6-v${VERSION}-macos-arm64.zip"
    EXTRACT_DIR="k6-v${VERSION}-macos-arm64"
    ;;
  Darwin-x86_64)
    ARCHIVE="k6-v${VERSION}-macos-amd64.zip"
    EXTRACT_DIR="k6-v${VERSION}-macos-amd64"
    ;;
  Linux-aarch64)
    ARCHIVE="k6-v${VERSION}-linux-arm64.tar.gz"
    EXTRACT_DIR="k6-v${VERSION}-linux-arm64"
    ;;
  Linux-x86_64)
    ARCHIVE="k6-v${VERSION}-linux-amd64.tar.gz"
    EXTRACT_DIR="k6-v${VERSION}-linux-amd64"
    ;;
  *)
    echo "unsupported platform for k6 install: ${OS}-${ARCH}" >&2
    exit 1
    ;;
esac

TARGET_BIN="${HOME}/.cargo/bin/k6"
if command -v k6 >/dev/null 2>&1; then
  CURRENT_VERSION="$(k6 version 2>/dev/null | awk 'NR==1 {print $2}')"
  if [[ "${CURRENT_VERSION}" == "v${VERSION}" || "${CURRENT_VERSION}" == "${VERSION}" ]]; then
    exit 0
  fi
fi

TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

URL="https://github.com/grafana/k6/releases/download/v${VERSION}/${ARCHIVE}"
curl -fsSL -o "${TMP_DIR}/${ARCHIVE}" "${URL}"

case "${ARCHIVE}" in
  *.zip)
    unzip -q "${TMP_DIR}/${ARCHIVE}" -d "${TMP_DIR}"
    ;;
  *.tar.gz)
    tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "${TMP_DIR}"
    ;;
esac

install -d "$(dirname "${TARGET_BIN}")"
install -m 755 "${TMP_DIR}/${EXTRACT_DIR}/k6" "${TARGET_BIN}"
