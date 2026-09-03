#!/usr/bin/env bash
set -euo pipefail

# Opt-in live-driver smoke for AGENT-007 / B-005.
#
# This intentionally exercises the real local Codex CLI session store. It does
# not copy or inspect ~/.codex session files. The runner may create normal
# local CLI session records, but its retained result and console output are
# verdict-only: raw session ids, paths, sentinels, driver output, and logs are
# never printed or retained by this smoke's artifact directory.

umask 077

safe_error() {
  printf '%s\n' "$1" >&2
}

validate_result_fields() {
  local run_alias="$1"
  local driver_result="$2"
  local direct_history_result="$3"
  local workspace_result="$4"
  local resume_result="$5"
  local verdict="$6"
  local digest="$7"

  [[ "${run_alias}" =~ ^run-[0-9a-f]{16}$ ]] || return 1
  case "${driver_result}" in completed|failed|session-unavailable|establishment-unconfirmed|transient-output-missing) ;; *) return 1 ;; esac
  case "${direct_history_result}" in not-observed|detected|not-run) ;; *) return 1 ;; esac
  case "${workspace_result}" in not-observed|detected|not-run) ;; *) return 1 ;; esac
  case "${resume_result}" in confirmed|failed|not-run) ;; *) return 1 ;; esac
  case "${verdict}" in PASS|FAIL) ;; *) return 1 ;; esac
  [[ "${digest}" =~ ^[0-9a-f]{16}$ && ! "${digest}" =~ ^0+$ ]]
}

render_sanitized_result() {
  local run_alias="$1"
  local driver_result="$2"
  local direct_history_result="$3"
  local workspace_result="$4"
  local resume_result="$5"
  local verdict="$6"
  local digest="$7"

  validate_result_fields "${run_alias}" "${driver_result}" "${direct_history_result}" \
    "${workspace_result}" "${resume_result}" "${verdict}" "${digest}" || return 1

  printf 'Run alias: %s\n' "${run_alias}"
  printf 'Driver: Codex CLI\n'
  printf 'Scope: local session persistence and cwd isolation\n'
  printf 'Session aliases: agent-a=session-a; agent-b=session-b\n'
  printf 'Driver invocation: %s\n' "${driver_result}"
  printf 'Direct-history cross-actor result: %s\n' "${direct_history_result}"
  printf 'Workspace cross-actor result: %s\n' "${workspace_result}"
  printf 'Resume-path result: %s\n' "${resume_result}"
  printf 'Verdict: %s\n' "${verdict}"
  printf 'Verdict summary SHA-256 (truncated): %s\n' "${digest}"
  printf 'Residual risk: no Choruz binding, provenance, routing, persistence, PTY, or fanout coverage\n'
}

public_result_template() {
  # Exercise the exact release-result renderer with static, valid opaque values.
  render_sanitized_result 'run-0123456789abcdef' 'completed' 'not-observed' \
    'not-observed' 'confirmed' 'PASS' '0123456789abcdef'
}

assert_public_result_template_is_safe() {
  local template
  template="$(public_result_template)"

  # These labels identify fields which would expose sensitive live-run data.
  # Keep the user-visible result strictly verdict-only.
  if grep -Fq -e 'Session IDs:' -e 'Workspace paths:' -e 'Artifacts:' \
    -e 'Raw CLI logs:' -e 'Synthetic sentinel' <<<"${template}"; then
    safe_error 'The public result template is not safe to retain.'
    return 1
  fi
}

if [[ "${CHORUZ_REAL_DRIVER_SMOKE_TEMPLATE_CHECK:-0}" == "1" ]]; then
  assert_public_result_template_is_safe
  public_result_template
  exit 0
fi

if [[ "${CHORUZ_REAL_DRIVER_SMOKE:-0}" != "1" ]]; then
  cat >&2 <<'MSG'
Refusing to run live-driver smoke without CHORUZ_REAL_DRIVER_SMOKE=1.

This command invokes a real local model driver and may create normal local CLI
session records. Run only with disposable sentinel values and workspaces.
MSG
  exit 2
fi

CODEX_BIN="${CHORUZ_CODEX_BINARY:-codex}"
if ! command -v "${CODEX_BIN}" >/dev/null 2>&1; then
  safe_error 'Codex CLI is not available.'
  exit 127
fi

timestamp="$(date +%Y%m%d%H%M%S)"
nonce="$(uuidgen 2>/dev/null | tr -d '-' | tr '[:upper:]' '[:lower:]' | cut -c1-8 || true)"
if [[ -z "${nonce}" ]]; then
  nonce="${RANDOM}${RANDOM}"
fi

raw_artifacts=()
created_directories=()

cleanup_created_artifacts() {
  local artifact
  local directory
  for artifact in ${raw_artifacts[@]+"${raw_artifacts[@]}"}; do
    rm -f -- "${artifact}" 2>/dev/null || :
  done
  for directory in ${created_directories[@]+"${created_directories[@]}"}; do
    rmdir -- "${directory}" 2>/dev/null || :
  done
}

canonicalize_directory() {
  (
    cd "$1" 2>/dev/null
    pwd -P
  )
}

if [[ -n "${CHORUZ_REAL_DRIVER_SMOKE_ROOT:-}" ]]; then
  requested_root="${CHORUZ_REAL_DRIVER_SMOKE_ROOT}"
  if [[ -e "${requested_root}" || -L "${requested_root}" ]] ||
    ! mkdir -m 700 -- "${requested_root}" 2>/dev/null; then
    safe_error 'Could not create a new configured disposable run directory.'
    exit 1
  fi
  trap cleanup_created_artifacts EXIT HUP INT TERM
  root="$(canonicalize_directory "${requested_root}")"
else
  root="$(mktemp -d "${TMPDIR:-/tmp}/choruz-agent007-codex.XXXXXX" 2>/dev/null || true)"
  if [[ -z "${root}" ]]; then
    safe_error 'Could not create a new disposable run directory.'
    exit 1
  fi
  trap cleanup_created_artifacts EXIT HUP INT TERM
  root="$(canonicalize_directory "${root}")"
fi

if [[ -z "${root}" || ! -d "${root}" || -L "${root}" ]]; then
  safe_error 'Could not verify the new disposable run directory.'
  exit 1
fi

workspace_a="${root}/isolation-agent-a"
workspace_b="${root}/isolation-agent-b"
if ! mkdir -m 700 -- "${workspace_a}" "${workspace_b}" 2>/dev/null; then
  safe_error 'Could not initialize the configured disposable run directory.'
  exit 1
fi
canonical_workspace_a="$(canonicalize_directory "${workspace_a}")"
canonical_workspace_b="$(canonicalize_directory "${workspace_b}")"
if [[ "${canonical_workspace_a}" != "${root}/isolation-agent-a" ||
  "${canonical_workspace_b}" != "${root}/isolation-agent-b" ||
  -L "${workspace_a}" || -L "${workspace_b}" ]]; then
  safe_error 'Could not verify disposable smoke workspaces.'
  exit 1
fi
workspace_a="${canonical_workspace_a}"
workspace_b="${canonical_workspace_b}"
created_directories=("${workspace_a}" "${workspace_b}")

a_direct="A_DIRECT_HISTORY_SENTINEL_${timestamp}_${nonce}"
a_establishment_challenge="A_WORKSPACE_SENTINEL_${timestamp}_${nonce}"
b_direct="B_DIRECT_HISTORY_SENTINEL_${timestamp}_${nonce}"
b_establishment_challenge="B_WORKSPACE_SENTINEL_${timestamp}_${nonce}"

a_sentinel_file="${workspace_a}/a-sentinel.txt"
b_sentinel_file="${workspace_b}/b-sentinel.txt"
raw_artifacts=("${a_sentinel_file}" "${b_sentinel_file}")
if ! { printf '%s\n' "${a_establishment_challenge}" > "${a_sentinel_file}"; } 2>/dev/null ||
  ! { printf '%s\n' "${b_establishment_challenge}" > "${b_sentinel_file}"; } 2>/dev/null; then
  safe_error 'Could not prepare disposable smoke inputs.'
  exit 1
fi

result_file="${root}/result.txt"
a_establish_out="${root}/agent-a-establish.txt"
b_establish_out="${root}/agent-b-establish.txt"
a_check_out="${root}/agent-a-check.txt"
b_check_out="${root}/agent-b-check.txt"
a_resume_out="${root}/agent-a-resume-check.txt"
b_resume_out="${root}/agent-b-resume-check.txt"

raw_artifacts=(
  "${a_sentinel_file}" "${b_sentinel_file}"
  "${a_establish_out}" "${b_establish_out}" "${a_check_out}" "${b_check_out}"
  "${a_resume_out}" "${b_resume_out}"
)

run_codex_establish() {
  local cwd="$1"
  local output_file="$2"
  local prompt="$3"
  local command_output
  local session_id

  # Codex reports the id on its command output. Keep that output only in this
  # process long enough to extract the id required for resume; never create a
  # raw CLI log file.
  if ! command_output="$("${CODEX_BIN}" exec \
    --skip-git-repo-check \
    --cd "${cwd}" \
    --sandbox read-only \
    --json \
    --output-last-message "${output_file}" \
    "${prompt}" 2>&1)"; then
    unset command_output
    return 1
  fi
  session_id="$(sed -nE 's/.*"thread_id"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' <<<"${command_output}" | head -n 1)"
  unset command_output
  if [[ -z "${session_id}" ]]; then
    return 2
  fi
  established_session_id="${session_id}"
  unset session_id
}

run_codex_resume() {
  local cwd="$1"
  local session_id="$2"
  local output_file="$3"
  local prompt="$4"

  (
    cd "${cwd}"
    "${CODEX_BIN}" exec resume \
      --skip-git-repo-check \
      --sandbox read-only \
      --output-last-message "${output_file}" \
      "${session_id}" \
      "${prompt}"
  ) >/dev/null 2>&1
}

contains_forbidden_token() {
  local file="$1"
  local forbidden_direct="$2"
  local forbidden_workspace="$3"

  [[ -f "${file}" ]] || return 2
  grep -Fq -- "${forbidden_direct}" "${file}" 2>/dev/null ||
    grep -Fq -- "${forbidden_workspace}" "${file}" 2>/dev/null
}

has_expected_establishment_output() {
  local file="$1"
  local expected="$2"

  # The establishment response is a file-only opaque challenge. Require the
  # exact reply so a generic acknowledgement cannot establish a false PASS.
  [[ -f "${file}" ]] && cmp -s "${file}" <(printf '%s\n' "${expected}")
}

has_all_transient_outputs() {
  local file
  for file in "$@"; do
    [[ -f "${file}" ]] || return 1
  done
}

truncated_sha256() {
  local summary="$1"
  local full_digest
  local digest

  if command -v shasum >/dev/null 2>&1; then
    full_digest="$(printf '%s' "${summary}" | shasum -a 256 | awk '{print $1}')"
  elif command -v sha256sum >/dev/null 2>&1; then
    full_digest="$(printf '%s' "${summary}" | sha256sum | awk '{print $1}')"
  else
    return 1
  fi

  if [[ ! "${full_digest}" =~ ^[0-9a-f]{64}$ ]]; then
    return 1
  fi
  digest="${full_digest:0:16}"
  if [[ "${digest}" =~ ^0+$ ]]; then
    return 1
  fi
  printf '%s\n' "${digest}"
}

write_sanitized_result() {
  local verdict="$1"
  local direct_history_result="$2"
  local workspace_result="$3"
  local resume_result="$4"
  local driver_result="$5"
  local summary
  local digest

  summary="driver=codex-cli;scope=local-session-persistence-and-cwd-isolation;run=${timestamp}-${nonce};driver-result=${driver_result};direct-history=${direct_history_result};workspace=${workspace_result};resume=${resume_result};verdict=${verdict}"
  if ! digest="$(truncated_sha256 "${summary}")"; then
    safe_error 'Could not create the sanitized verdict digest.'
    return 1
  fi

  if ! render_sanitized_result "run-${digest}" "${driver_result}" \
    "${direct_history_result}" "${workspace_result}" "${resume_result}" \
    "${verdict}" "${digest}" > "${result_file}" 2>/dev/null; then
    safe_error 'Could not write the sanitized smoke result.'
    return 1
  fi

  if ! cat "${result_file}" 2>/dev/null; then
    safe_error 'Could not display the sanitized smoke result.'
    return 1
  fi
}

assert_public_result_template_is_safe
printf '%s\n' 'Running Codex live-driver session isolation smoke in the configured disposable run directory.'

verdict='PASS'
driver_result='completed'
direct_history_result='not-observed'
workspace_result='not-observed'
resume_result='confirmed'

if ! run_codex_establish "${workspace_a}" "${a_establish_out}" \
  "You are isolation-agent-a for a Choruz AGENT-007 release smoke. Record this direct-chat sentinel in this Codex session only: ${a_direct}. Read ./a-sentinel.txt and return exactly A_ESTABLISHED:<file value>. Do not add an explanation."; then
  verdict='FAIL'
  driver_result='failed'
  direct_history_result='not-run'
  workspace_result='not-run'
  resume_result='not-run'
else
  # Session ids remain in process memory only and are used exclusively for the
  # resume invocations below. They are never written to the result or console.
  session_a="${established_session_id}"
  unset established_session_id

  if ! run_codex_establish "${workspace_b}" "${b_establish_out}" \
    "You are isolation-agent-b for a Choruz AGENT-007 release smoke. Record this direct-chat sentinel in this Codex session only: ${b_direct}. Read ./b-sentinel.txt and return exactly B_ESTABLISHED:<file value>. Do not add an explanation."; then
    verdict='FAIL'
    driver_result='failed'
    direct_history_result='not-run'
    workspace_result='not-run'
    resume_result='not-run'
  else
    session_b="${established_session_id}"
    unset established_session_id
  fi

  if [[ "${verdict}" == 'PASS' ]] &&
    { ! has_expected_establishment_output "${a_establish_out}" "A_ESTABLISHED:${a_establishment_challenge}" ||
      ! has_expected_establishment_output "${b_establish_out}" "B_ESTABLISHED:${b_establishment_challenge}"; }; then
    verdict='FAIL'
    driver_result='establishment-unconfirmed'
    direct_history_result='not-run'
    workspace_result='not-run'
    resume_result='not-run'
  fi

  if [[ "${verdict}" == 'PASS' ]] &&
    { ! run_codex_resume "${workspace_b}" "${session_b}" "${b_check_out}" \
      "Isolation check: search only your current conversation context and current workspace for any complete token beginning with A_DIRECT_HISTORY_SENTINEL_ or A_WORKSPACE_SENTINEL_. Do not guess and do not invent suffixes. If you can see a complete token with either prefix, answer exactly LEAK_DETECTED:<the complete token>. If you cannot see a complete token with either prefix, answer exactly ISOLATION_OK_B_CANNOT_SEE_A." ||
      ! run_codex_resume "${workspace_a}" "${session_a}" "${a_check_out}" \
      "Isolation check: search only your current conversation context and current workspace for any complete token beginning with B_DIRECT_HISTORY_SENTINEL_ or B_WORKSPACE_SENTINEL_. Do not guess and do not invent suffixes. If you can see a complete token with either prefix, answer exactly LEAK_DETECTED:<the complete token>. If you cannot see a complete token with either prefix, answer exactly ISOLATION_OK_A_CANNOT_SEE_B." ||
      ! run_codex_resume "${workspace_b}" "${session_b}" "${b_resume_out}" \
      "Resume-path isolation check: search only your current conversation context and current workspace for complete tokens beginning with A_DIRECT_HISTORY_SENTINEL_ or A_WORKSPACE_SENTINEL_. If you can see a complete token with either prefix, answer exactly LEAK_DETECTED:<the complete token>. If you cannot see either complete token, answer exactly ISOLATION_OK_B_RESUME_CANNOT_SEE_A." ||
      ! run_codex_resume "${workspace_a}" "${session_a}" "${a_resume_out}" \
      "Resume-path isolation check: search only your current conversation context and current workspace for complete tokens beginning with B_DIRECT_HISTORY_SENTINEL_ or B_WORKSPACE_SENTINEL_. If you can see a complete token with either prefix, answer exactly LEAK_DETECTED:<the complete token>. If you cannot see either complete token, answer exactly ISOLATION_OK_A_RESUME_CANNOT_SEE_B."; }; then
    verdict='FAIL'
    driver_result='failed'
    direct_history_result='not-run'
    workspace_result='not-run'
    resume_result='not-run'
  fi

  if [[ "${verdict}" == 'PASS' ]] && ! has_all_transient_outputs \
    "${a_check_out}" "${b_check_out}" "${a_resume_out}" "${b_resume_out}"; then
    verdict='FAIL'
    driver_result='transient-output-missing'
    direct_history_result='not-run'
    workspace_result='not-run'
    resume_result='not-run'
  fi

  if [[ "${verdict}" == 'PASS' ]] &&
    { contains_forbidden_token "${b_check_out}" "${a_direct}" "${a_establishment_challenge}" ||
      contains_forbidden_token "${b_resume_out}" "${a_direct}" "${a_establishment_challenge}" ||
      contains_forbidden_token "${a_check_out}" "${b_direct}" "${b_establishment_challenge}" ||
      contains_forbidden_token "${a_resume_out}" "${b_direct}" "${b_establishment_challenge}"; }; then
    verdict='FAIL'
    direct_history_result='detected'
    workspace_result='detected'
  fi

  if [[ "${verdict}" == 'PASS' ]] &&
    { ! has_expected_establishment_output "${b_check_out}" 'ISOLATION_OK_B_CANNOT_SEE_A' ||
      ! has_expected_establishment_output "${a_check_out}" 'ISOLATION_OK_A_CANNOT_SEE_B' ||
      ! has_expected_establishment_output "${b_resume_out}" 'ISOLATION_OK_B_RESUME_CANNOT_SEE_A' ||
      ! has_expected_establishment_output "${a_resume_out}" 'ISOLATION_OK_A_RESUME_CANNOT_SEE_B'; }; then
    verdict='FAIL'
    resume_result='failed'
  fi
fi

if ! write_sanitized_result "${verdict}" "${direct_history_result}" "${workspace_result}" "${resume_result}" "${driver_result}"; then
  exit 1
fi

if [[ "${verdict}" != 'PASS' ]]; then
  safe_error 'Smoke failed. Review only the sanitized result in the configured disposable run directory.'
  exit 1
fi
