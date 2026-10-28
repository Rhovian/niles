use anyhow::{Result, ensure};

use super::{AgentInvocation, SystemPromptArg};

const MAX_PRIMARY_BYTES: usize = 65_536;
const MAX_EXEC_BYTES: usize = 131_072;

impl AgentInvocation {
    pub fn pass_primary(&mut self, text: &str) -> Result<()> {
        ensure!(
            text.len() <= MAX_PRIMARY_BYTES,
            "primary skill is {} bytes; maximum is 65536 bytes",
            text.len()
        );
        ensure!(
            !text.contains('\0'),
            "primary skill contains NUL ({} bytes)",
            text.len()
        );
        match self.system_prompt {
            SystemPromptArg::Flag(flag) => self.args.extend([flag.to_string(), text.to_owned()]),
            SystemPromptArg::Config { flag, key } => {
                let value = format!("{key}={}", toml_basic_string(text));
                // Linux MAX_ARG_STRLEN includes the terminating NUL; TOML escaping can
                // grow a 64 KiB body sixfold. Check the actual exec argument, not shell quoting.
                ensure!(
                    value.len() < MAX_EXEC_BYTES,
                    "encoded primary skill is {} bytes; maximum argument is 131071 bytes",
                    value.len()
                );
                self.args.extend([flag.to_string(), value]);
            }
        }
        Ok(())
    }
}

// Codex 0.162.1 parses --config values as TOML, then falls back to raw strings:
// https://github.com/openai/codex/blob/rust-v0.162.1/codex-rs/utils/cli/src/config_override.rs
// No TOML serializer is a dependency; encode basic strings per TOML 1.0.0 §String.
fn toml_basic_string(text: &str) -> String {
    let mut encoded = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => encoded.push_str("\\\\"),
            '"' => encoded.push_str("\\\""),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            ch if ch.is_ascii_control() => encoded.push_str(&format!("\\u{:04X}", u32::from(ch))),
            ch => encoded.push(ch),
        }
    }
    encoded.push('"');
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{InvocationDefaults, ModelRoster, invocation};

    #[test]
    fn toml_strings_escape_metacharacters_controls_and_preserve_unicode() {
        assert_eq!(
            toml_basic_string("\\\"\n\r\t\u{08}\u{0c}\u{1b}\u{7f}é"),
            "\"\\\\\\\"\\n\\r\\t\\u0008\\u000C\\u001B\\u007Fé\""
        );
        assert_eq!(toml_basic_string(""), "\"\"");
    }

    #[test]
    fn limits_reject_before_mutation_and_allow_the_raw_boundary() {
        let models = ModelRoster::builtin().unwrap();
        let mut agent = invocation("codex", InvocationDefaults::Worker, &models).unwrap();
        let args = agent.args.clone();
        let env = agent.env.clone();
        for text in ["bad\0text".into(), "é".repeat(MAX_PRIMARY_BYTES / 2 + 1)] {
            assert!(agent.pass_primary(&text).is_err());
            assert_eq!(agent.args, args);
            assert_eq!(agent.env, env);
        }
        agent.pass_primary(&"x".repeat(MAX_PRIMARY_BYTES)).unwrap();
        let count = (MAX_EXEC_BYTES - 1 - "developer_instructions=".len() - 2) / 2;
        agent.pass_primary(&"\"".repeat(count)).unwrap();
        assert!(agent.args.last().unwrap().len() < MAX_EXEC_BYTES);
        let before = agent.args.clone();
        assert!(agent.pass_primary(&"\"".repeat(count + 1)).is_err());
        assert_eq!(agent.args, before);
    }
}
