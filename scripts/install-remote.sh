#!/bin/bash
set -eu

root=$(cd -- "${BASH_SOURCE[0]%/*}/.." && pwd)
(cd "$root/apps/pwa" && pnpm install --frozen-lockfile && pnpm build)
cargo install --path "$root" --locked --features remote
printf 'Web shell: %s/apps/pwa/dist\n' "$root"
