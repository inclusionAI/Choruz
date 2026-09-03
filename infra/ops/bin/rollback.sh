#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RELEASES_DIR="${ROOT_DIR}/releases"
CURRENT_LINK="${RELEASES_DIR}/current"
PREVIOUS_LINK="${RELEASES_DIR}/previous"

preflight_target() {
  test -x "${TARGET}/bin/api-gateway" && \
    test -x "${TARGET}/bin/choruz-pipeline" && \
    test -f "${TARGET}/web/apps/web/server.js" && \
    test -d "${TARGET}/web/apps/web/.next/static"
}

set_current_target() {
  local target="$1"
  local pending_link="${CURRENT_LINK}.next"
  rm -f "${pending_link}"
  ln -s "${target}" "${pending_link}"
  rm -f "${CURRENT_LINK}"
  mv -f "${pending_link}" "${CURRENT_LINK}"
}

restart_services() {
  local failed=0 service
  case "$(uname -s)" in
    Darwin)
      for service in choruz-api-gateway pipeline web-app; do
        launchctl kickstart -k "gui/$(id -u)/com.choruz.${service}" || failed=1
      done
      ;;
    Linux)
      sudo systemctl daemon-reload || failed=1
      for service in choruz-api-gateway pipeline web-app; do
        sudo systemctl restart "choruz-${service}.service" || failed=1
      done
      ;;
    *)
      echo "unsupported platform for managed rollback" >&2
      return 1
      ;;
  esac
  return "${failed}"
}

services_healthy() {
  local service
  case "$(uname -s)" in
    Darwin)
      for service in choruz-api-gateway pipeline web-app; do
        launchctl print "gui/$(id -u)/com.choruz.${service}" | grep -q 'state = running' || return 1
      done
      ;;
    Linux)
      for service in choruz-api-gateway pipeline web-app; do
        systemctl is-active --quiet "choruz-${service}.service" || return 1
      done
      ;;
    *) return 1 ;;
  esac
}

main() {
  if [[ ! -L "${PREVIOUS_LINK}" ]]; then
    echo "no previous release available" >&2
    exit 1
  fi

  TARGET="$(readlink "${PREVIOUS_LINK}")"
  KNOWN_GOOD="$(readlink "${CURRENT_LINK}" 2>/dev/null || true)"
  if [[ -z "${KNOWN_GOOD}" ]]; then
    echo "current release target is unavailable" >&2
    exit 1
  fi

  if ! preflight_target; then
    echo "rollback target is missing a required Choruz release artifact" >&2
    exit 1
  fi

  set_current_target "${TARGET}"
  if ! restart_services || ! services_healthy; then
    echo "rollback restart or health check failed; restoring ${KNOWN_GOOD}" >&2
    set_current_target "${KNOWN_GOOD}"
    if restart_services && services_healthy; then
      echo "restored known-good release ${KNOWN_GOOD}; manual investigation required" >&2
    else
      echo "CRITICAL: failed to restore known-good release ${KNOWN_GOOD}; services may be in a mixed state" >&2
    fi
    exit 1
  fi

  echo "rolled back to ${TARGET}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
