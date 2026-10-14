#!/usr/bin/env bash
# Builds every platform's package on GitHub Actions (the `package` job of ci.yml) for the
# current branch, waits for it, and downloads the results into dist/.
# Needs `gh` (installed by mise) and `gh auth login`.
set -euo pipefail

branch="$(git rev-parse --abbrev-ref HEAD)"
head="$(git rev-parse HEAD)"
upstream="$(git rev-parse --verify --quiet "@{u}" || true)"
if [[ "$upstream" != "$head" ]]; then
  echo "HEAD ($head) isn't pushed to the remote; CI builds what's on GitHub. Push first." >&2
  exit 1
fi

gh workflow run ci.yml --ref "$branch"
echo "Waiting for the run to start..."
run_id=""
for _ in $(seq 30); do
  run_id="$(gh run list --workflow ci.yml --event workflow_dispatch --branch "$branch" --commit "$head" \
    --limit 1 --json databaseId --jq '.[0].databaseId // empty')"
  [[ -n "$run_id" ]] && break
  sleep 2
done
[[ -n "$run_id" ]] || { echo "the workflow run didn't show up" >&2; exit 1; }

gh run watch "$run_id" --exit-status
rm -rf dist
gh run download "$run_id" --dir dist
# Artifacts download into one folder each; flatten them.
find dist -mindepth 2 -type f -exec mv {} dist/ \;
find dist -mindepth 1 -type d -empty -delete
ls -l dist
