#!/usr/bin/env bash
set -eu

# Line-based Rust checks exclude fixtures and ordinary quoted comment markers.
# Identity defaults flatten newlines before grep; those matches report filenames.
# Multiline arms, unusual quoting, and defaults containing semicolons remain heuristic.

comments='(^[[:space:]]*(//+|/\*+|\*)|[;{}][[:space:]]*(//+|/\*+))(.*[^[:alnum:]_])?(for[[:space:]]+now|workaround|todo|fixme|hack)([^[:alnum:]_]|$)'
identity_function='\.map_or\([^;]*,[[:space:]]*([[:alpha:]_][[:alnum:]_]*::)*identity[[:space:]]*,?[[:space:]]*\)'
identity_closure='\.map_or\([^;]*,[[:space:]]*(move[[:space:]]+)?\|[[:space:]]*(mut[[:space:]]+)?([[:alpha:]_][[:alnum:]_]*)[[:space:]]*\|[[:space:]]*\{?[[:space:]]*\3[[:space:]]*\}?[[:space:]]*,?[[:space:]]*\)'
failed=0
for pattern in \
    "$comments" \
    '(^|[^[:alnum:]_])_[[:space:]]*=>[[:space:]]*(\{[[:space:]]*\}|\(\))' \
    '(^|[^[:alnum:]_])unsupported([^[:alnum:]_]|$)'
do
    if grep -rniE --include='*.rs' --exclude='fixtures.rs' --exclude-dir=fixtures \
        -- "$pattern" src tests build.rs; then
        failed=1
    else
        result=$?
        if [ "$result" -ne 1 ]; then
            exit "$result"
        fi
    fi
done
while IFS= read -r file; do
    if tr '\n' ' ' < "$file" | grep -qE -e "$identity_function" -e "$identity_closure"; then
        printf '%s: map_or with identity\n' "$file"
        failed=1
    else
        result=$?
        if [ "$result" -ne 1 ]; then exit "$result"; fi
    fi
done < <(find src tests build.rs -type f -name '*.rs' ! -name fixtures.rs ! -path '*/fixtures/*')
exit "$failed"
