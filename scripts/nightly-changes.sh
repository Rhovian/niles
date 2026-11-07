#!/usr/bin/env bash
set -euo pipefail

gh api --paginate --slurp "repos/$GITHUB_REPOSITORY/releases" > "$RUNNER_TEMP/nightly-releases.json"
jq -r '[.[][] | select(.prerelease and (.draft | not) and (.tag_name | contains("-nightly.")))]
  | max_by(.published_at)
  | if . == null then true
    else (.target_commitish | if type != "string" or length == 0 then error("nightly release has no target commit")
      else . != env.GITHUB_SHA end)
    end' "$RUNNER_TEMP/nightly-releases.json"
