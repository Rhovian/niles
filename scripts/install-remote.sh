#!/bin/bash
set -eu

root=$(cd -- "${BASH_SOURCE[0]%/*}/.." && pwd)
(cd "$root/web" && pnpm install --frozen-lockfile && pnpm build)
cargo install --path "$root" --locked --features remote
printf 'Web shell: %s/web/dist\n' "$root"
