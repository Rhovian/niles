use super::{Adapter, Capabilities, Prompt, operations::InputDisposition};
use crate::tmux::{self, TmuxTarget};
use anyhow::Result;

/// Native CLIs expose prompt delivery without structured lifecycle negotiation.
pub(crate) struct NativeTui {
    pub(crate) target: TmuxTarget,
}

impl Adapter for NativeTui {
    fn negotiate(&mut self) -> Result<Capabilities> {
        Ok(Capabilities::default())
    }
    fn prompt(&mut self, prompt: Prompt) -> Result<InputDisposition> {
        tmux::send_line(&self.target, &prompt.0)?;
        Ok(InputDisposition::Unconfirmed)
    }
}
