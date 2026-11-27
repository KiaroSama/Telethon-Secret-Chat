#!/usr/bin/env bash
# Shared CI logging: fixed diagnostics only; never pass credentials to log().
log() {
  local severity="$1"
  shift
  case "$severity" in
    INFO|WARNING|ERROR) ;;
    DEBUG) [[ "${CI_DEBUG:-0}" == 1 ]] || return 0 ;;
    *) printf '%s [ERROR] Invalid CI log severity\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" >&2; return 2 ;;
  esac
  if [[ "$severity" == ERROR ]]; then
    printf '%s [%s] %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" "$severity" "$*" >&2
  else
    printf '%s [%s] %s\n' "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" "$severity" "$*"
  fi
}
