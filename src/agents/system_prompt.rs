use anyhow::{Result, ensure};

use super::{AgentInvocation, SystemPromptArg};

impl AgentInvocation {
    pub fn pass_primary(&mut self, text: &str) -> Result<()> {
        ensure!(!text.contains('\0'), "primary skill contains NUL");
        match self.system_prompt {
            SystemPromptArg::Flag(flag) => self.args.extend([flag.to_string(), text.to_owned()]),
            SystemPromptArg::Config { flag, key } => {
                let value = format!("{key}={}", toml_basic_string(text));
                self.args.extend([flag.to_string(), value]);
            }
            SystemPromptArg::Env(name) => self.env.push((name.to_string(), text.to_owned())),
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
    fn nul_is_rejected_before_mutating_builtin_launches() {
        let models = ModelRoster::builtin().unwrap();
        for family in ["claude", "codex", "hermes", "pi"] {
            let mut agent = invocation(family, InvocationDefaults::Worker, &models).unwrap();
            let args = agent.args.clone();
            let env = agent.env.clone();
            assert!(agent.pass_primary("bad\0text").is_err());
            assert_eq!(agent.args, args);
            assert_eq!(agent.env, env);
        }
    }
}
