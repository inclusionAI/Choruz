#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RELEASES_DIR="${ROOT_DIR}/releases"
TIMESTAMP="$(date +"%Y%m%d%H%M%S")"
RELEASE_DIR="${RELEASES_DIR}/${TIMESTAMP}"
CURRENT_LINK="${RELEASES_DIR}/current"
PREVIOUS_LINK="${RELEASES_DIR}/previous"

SERVICES=(
  choruz-api-gateway
  pipeline
  web-app
)

restart_services() {
  case "$(uname -s)" in
    Darwin)
      for service in "${SERVICES[@]}"; do
        launchctl kickstart -k "gui/$(id -u)/com.choruz.${service}"
      done
      ;;
    Linux)
      sudo systemctl daemon-reload
      for service in "${SERVICES[@]}"; do
        sudo systemctl restart "choruz-${service}.service"
      done
      ;;
    *)
      echo "unsupported platform for managed restart" >&2
      ;;
  esac
}

package_release() {
  mkdir -p "${RELEASE_DIR}/bin" "${RELEASE_DIR}/web"

  cargo build --release -p choruz-api-gateway -p choruz-pipeline -p choruz-connector

  pnpm install --frozen-lockfile
  pnpm web:build

  cp "${ROOT_DIR}/target/release/choruz-api-gateway" "${RELEASE_DIR}/bin/"
  cp "${ROOT_DIR}/target/release/choruz-pipeline" "${RELEASE_DIR}/bin/"
  cp "${ROOT_DIR}/target/release/choruz-connector" "${RELEASE_DIR}/bin/"
  cp -R "${ROOT_DIR}/apps/web/.next/standalone/." "${RELEASE_DIR}/web/"
  mkdir -p "${RELEASE_DIR}/web/apps/web/.next"
  cp -R "${ROOT_DIR}/apps/web/.next/static" "${RELEASE_DIR}/web/apps/web/.next/static"
  cp "${ROOT_DIR}/apps/web/package.json" "${RELEASE_DIR}/web/package.json"
  cp "${ROOT_DIR}/apps/web/next.config.ts" "${RELEASE_DIR}/web/next.config.ts"
  cp -R "${ROOT_DIR}/apps/web/public" "${RELEASE_DIR}/web/public" 2>/dev/null || true
  mkdir -p "${RELEASE_DIR}/infra"
  cp -R "${ROOT_DIR}/infra/ops" "${RELEASE_DIR}/infra/"

  test -f "${RELEASE_DIR}/web/apps/web/server.js" || {
    echo "packaged web entrypoint is missing" >&2
    exit 1
  }
  test -d "${RELEASE_DIR}/web/apps/web/.next/static" || {
    echo "packaged web static assets are missing" >&2
    exit 1
  }
  test -x "${RELEASE_DIR}/bin/choruz-connector" || {
    echo "packaged Runtime Connector is missing" >&2
    exit 1
  }

  if [[ -L "${CURRENT_LINK}" ]]; then
    rm -f "${PREVIOUS_LINK}"
    ln -s "$(readlink "${CURRENT_LINK}")" "${PREVIOUS_LINK}"
  fi

  rm -f "${CURRENT_LINK}"
  ln -s "${RELEASE_DIR}" "${CURRENT_LINK}"
}

case "${1:-package}" in
  package)
    package_release
    ;;
  deploy)
    package_release
    restart_services
    ;;
  restart)
    restart_services
    ;;
  *)
    echo "usage: $0 [package|deploy|restart]" >&2
    exit 1
    ;;
esac
