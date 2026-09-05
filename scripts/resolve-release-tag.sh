#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: scripts/resolve-release-tag.sh OWNER/REPOSITORY TAG" >&2
  exit 2
fi

stratadiff_repository=$1
stratadiff_release_tag=$2
if [[ ! "${stratadiff_repository}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "repository must use OWNER/REPOSITORY form" >&2
  exit 1
fi
if [[ ! "${stratadiff_release_tag}" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "release tag must be a stable vMAJOR.MINOR.PATCH tag: ${stratadiff_release_tag}" >&2
  exit 1
fi

stratadiff_object="$(
  gh api \
    "repos/${stratadiff_repository}/git/ref/tags/${stratadiff_release_tag}" \
    --jq '.object.type + "\t" + .object.sha'
)"

for _ in 1 2 3 4 5 6 7 8; do
  stratadiff_object_type=${stratadiff_object%%$'\t'*}
  stratadiff_object_sha=${stratadiff_object#*$'\t'}
  if [[ "${stratadiff_object_type}" == "${stratadiff_object}" || \
        ! "${stratadiff_object_sha}" =~ ^[0-9a-f]{40}$ ]]; then
    echo "GitHub returned an invalid object for tag ${stratadiff_release_tag}" >&2
    exit 1
  fi
  case "${stratadiff_object_type}" in
    commit)
      printf '%s\n' "${stratadiff_object_sha}"
      exit 0
      ;;
    tag)
      stratadiff_object="$(
        gh api \
          "repos/${stratadiff_repository}/git/tags/${stratadiff_object_sha}" \
          --jq '.object.type + "\t" + .object.sha'
      )"
      ;;
    *)
      echo "tag ${stratadiff_release_tag} points to unsupported object type ${stratadiff_object_type}" >&2
      exit 1
      ;;
  esac
done

echo "tag ${stratadiff_release_tag} exceeds the eight-object dereference limit" >&2
exit 1
