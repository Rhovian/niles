use super::write_executable;
use std::path::Path;

pub fn write_catalogues(bin: &Path) {
    for (family, args, request, body) in [
        (
            "codex",
            "debug models",
            "",
            include_str!("../fixtures/catalogues/codex.json"),
        ),
        (
            "claude",
            "-p --input-format stream-json --output-format stream-json --verbose --no-session-persistence --strict-mcp-config --setting-sources user",
            "initialize",
            include_str!("../fixtures/catalogues/claude.json"),
        ),
        (
            "pi",
            "--mode rpc --no-session --no-extensions",
            "get_available_models",
            include_str!("../fixtures/catalogues/pi.json"),
        ),
    ] {
        let read = if request.is_empty() {
            String::new()
        } else {
            format!(
                "read request || exit 1\ncase \"$request\" in *{request}*) ;; *) exit 2 ;; esac\ncat >/dev/null\n"
            )
        };
        let notices = if family != "claude" {
            ""
        } else {
            "printf '%s\\n' '{\"type\":\"system\"}' '{\"type\":\"system\"}' || exit 5\n"
        };
        write_executable(
            &bin.join(family),
            &format!(
                "#!/bin/sh\n[ \"${{0##*/}}\" = '{family}' ] || exit 6\ncase \"$1\" in debug|-p|--mode)\n[ \"$*\" = '{args}' ] || exit 3\n[ ! -e .catalogue-repo-marker ] || exit 4\n{read}{notices}printf '%s\\n' '{}'\n;;\nesac\nexit 0\n",
                body.trim()
            ),
        );
    }
}
