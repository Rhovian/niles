#!/bin/bash
set -eu

mode=check
case $#:$* in
    0:) ;;
    1:--update) mode=update ;;
    1:--plan) mode=plan ;;
    *) printf 'Usage: %s [--update | --plan]\n' "$0" >&2; exit 2 ;;
esac
if ! command -v gh >/dev/null; then
    printf 'ERROR: GitHub CLI (gh) is required\n' >&2
    exit 2
fi
root=$(cd -- "${BASH_SOURCE[0]%/*}/.." && pwd)
profiles="$root/src/agents/families.rs"

tested_version() {
    local agent=$1 line id='' value='' count=0 in_profiles=0
    local id_pattern='^[[:space:]]*id:[[:space:]]*"([^"]+)",'
    local version_pattern='^[[:space:]]*tested_version:[[:space:]]*"([^"]+)",'
    while IFS= read -r line; do
        if [[ $line == 'const PROFILES: &[AgentProfile] = &[' ]]; then
            in_profiles=1
            continue
        fi
        (( in_profiles )) || continue
        [[ $line != '];' ]] || break
        if [[ $line =~ $id_pattern ]]; then
            id=${BASH_REMATCH[1]}
        elif [[ $id == "$agent" && $line =~ $version_pattern ]]; then
            value=${BASH_REMATCH[1]}
            count=$((count + 1))
        fi
    done < "$profiles"
    if (( count != 1 )); then
        printf 'ERROR: cannot read one tested version for %s from %s\n' "$agent" "$profiles" >&2
        return 1
    fi
    printf '%s' "$value"
}

stable_version() {
    local version=${1#rust-v}
    version=${version#v}
    local pattern='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
    if [[ ! $version =~ $pattern ]]; then
        printf 'ERROR: expected stable major.minor.patch, got %s\n' "$1" >&2
        return 1
    fi
    printf '%s' "$version"
}

newer_than() {
    # gh exposes release metadata, not version ordering:
    # https://cli.github.com/manual/gh_release_view
    local -a latest_parts tested_parts
    local i
    IFS=. read -r -a latest_parts <<< "$1"
    IFS=. read -r -a tested_parts <<< "$2"
    for i in 0 1 2; do
        if (( latest_parts[i] > tested_parts[i] )); then return 0; fi
        if (( latest_parts[i] < tested_parts[i] )); then return 1; fi
    done
    return 1
}

detect_update() {
    local cli=$1 output=$2 latest=$3 executable npm_root package directory origin published
    update_command=()
    remove_command=()
    executable=$(command -v "$cli") || return 1
    if [[ $cli == hermes && $output == *$'\nInstall method: git\n'* ]]; then
        directory=${output#*$'\nInstall directory: '}
        directory=${directory%%$'\n'*}
        origin=$(git -C "$directory" remote get-url origin) || return 1
        case $origin in
            https://github.com/NousResearch/hermes-agent|https://github.com/NousResearch/hermes-agent.git|git@github.com:NousResearch/hermes-agent.git) ;;
            *) printf 'ERROR: Hermes checkout is not the official repository: %s\n' "$origin" >&2; return 1 ;;
        esac
        # The native updater owns dependency preparation and service restarts.
        # --channel stable and --yes are documented by `hermes update --help`.
        method=git
        update_command=("$executable" update --yes --channel stable)
        return 0
    fi
    if command -v npm >/dev/null; then
        # npm root identifies its global tree, not the owner of a PATH binary:
        # https://docs.npmjs.com/cli/v11/commands/npm-root
        npm_root=$(npm root --global) || return 1
        npm_root=$(realpath "$npm_root") || return 1
        executable=$(realpath "$executable") || return 1
        case $cli:$executable in
            claude:"$npm_root/@anthropic-ai/claude-code/"*) package=@anthropic-ai/claude-code ;;
            codex:"$npm_root/@openai/codex/"*) package=@openai/codex ;;
            pi:"$npm_root/@earendil-works/pi-coding-agent/"*) package=@earendil-works/pi-coding-agent ;;
            pi:"$npm_root/@mariozechner/pi-coding-agent/"*) package=@mariozechner/pi-coding-agent ;;
            *) printf 'ERROR: cannot identify a supported install method for %s (%s)\n' "$cli" "$executable" >&2; return 1 ;;
        esac
        npm_prefix=$(npm prefix --global) || return 1
        method="npm global ($package)"
        if [[ $package == @mariozechner/pi-coding-agent ]]; then
            # The old scope stops at 0.73.1; latest Pi requires its renamed package.
            # https://pi.dev/changelog/2026/5/7/pi-has-a-new-home
            # pi update cannot pin a GitHub release; preflight the new package
            # before removing the old one, rather than relying on npm's dist-tag.
            remove_command=(npm uninstall --global --prefix "$npm_prefix" "$package")
            package=@earendil-works/pi-coding-agent
        fi
        published=$(npm view "$package@$latest" version) || return 1
        if [[ $published != "$latest" ]]; then
            printf 'ERROR: GitHub latest %s is unavailable from npm for %s\n' "$latest" "$package" >&2
            return 1
        fi
        update_command=(npm install --global --prefix "$npm_prefix" "$package@$latest")
        return 0
    fi
    printf 'ERROR: cannot identify a supported install method for %s\n' "$cli" >&2
    return 1
}

print_command() {
    printf '%q' "$1"
    shift
    if (( $# )); then printf ' %q' "$@"; fi
    printf '\n'
}

# Bash arrays contain words, not nested argv; offsets retain command boundaries.
# https://www.gnu.org/software/bash/manual/html_node/Arrays.html
update_argv=() update_offsets=() update_lengths=() update_clis=()
queue_update() {
    update_clis+=("$1")
    shift
    update_offsets+=("${#update_argv[@]}")
    update_lengths+=("$#")
    update_argv+=("$@")
}
# gh returns release names, not the embedded CLI version (Hermes tags can be dates):
# https://cli.github.com/manual/gh_release_view
version_pattern='(^|[[:space:]])v?([0-9]+\.[0-9]+\.[0-9]+([-+][[:alnum:].+-]+)?)'
exit_code=0
printf 'cli\tinstalled\ttested\tlatest\trepo\tresult\n'
for cli in codex claude hermes pi; do
    case $cli in
        codex) repo=openai/codex ;;
        claude) repo=anthropics/claude-code ;;
        hermes) repo=NousResearch/hermes-agent ;;
        pi) repo=earendil-works/pi ;;
    esac
    installed=missing
    output=''
    if command -v "$cli" >/dev/null; then
        if output=$("$cli" --version 2>&1); then
            first_line=${output%%$'\n'*}
            if [[ $first_line =~ $version_pattern ]]; then
                installed=${BASH_REMATCH[2]}
            else
                printf 'ERROR: %s --version output unrecognized: %s\n' "$cli" "$output" >&2
                installed=error
                exit_code=2
            fi
        else
            printf 'ERROR: %s --version failed: %s\n' "$cli" "$output" >&2
            installed=error
            exit_code=2
        fi
    fi
    tested=error
    if ! raw=$(tested_version "$cli") || ! tested=$(stable_version "$raw"); then
        printf 'ERROR: %s tested version lookup failed\n' "$cli" >&2
        tested=error
        exit_code=2
    fi
    latest=error
    result=error
    if name=$(gh release view --repo "$repo" --json name --jq .name); then
        if [[ $name =~ $version_pattern ]] && latest=$(stable_version "${BASH_REMATCH[2]}"); then
            if [[ $tested != error ]]; then
                result=covered
                if newer_than "$latest" "$tested"; then
                    result=newer
                    if (( exit_code == 0 )); then exit_code=1; fi
                fi
            fi
        else
            printf 'ERROR: %s latest release name unrecognized (%s)\n' "$cli" "$repo" >&2
            latest=error
            exit_code=2
        fi
    else
        printf 'ERROR: %s GitHub latest release lookup failed (%s)\n' "$cli" "$repo" >&2
        exit_code=2
    fi
    if [[ $installed == error || $tested == error || $latest == error ]]; then result=error; fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$cli" "$installed" "$tested" "$latest" "$repo" "$result"
    if [[ $mode != check ]]; then
        if [[ $installed == missing || $installed == error || $latest == error ]]; then
            printf 'ERROR: cannot plan an update for %s without installed/latest versions\n' "$cli" >&2
            exit_code=2
        elif detect_update "$cli" "$output" "$latest"; then
            printf '%s method: %s\n' "$cli" "$method"
            if (( ${#remove_command[@]} )); then
                print_command "${remove_command[@]}"
                queue_update "$cli" "${remove_command[@]}"
            fi
            print_command "${update_command[@]}"
            queue_update "$cli" "${update_command[@]}"
        else
            printf 'ERROR: %s update planning failed\n' "$cli" >&2
            exit_code=2
        fi
    fi
done
if [[ $mode == update ]]; then
    # No installation changes until all four release and method lookups succeed.
    if (( exit_code == 2 )); then exit 2; fi
    for i in "${!update_offsets[@]}"; do
        update_command=("${update_argv[@]:${update_offsets[i]}:${update_lengths[i]}}")
        if ! "${update_command[@]}"; then
            printf 'ERROR: %s update failed: ' "${update_clis[i]}" >&2
            print_command "${update_command[@]}" >&2
            exit 2
        fi
    done
fi
exit "$exit_code"
