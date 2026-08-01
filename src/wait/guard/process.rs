use anyhow::{Context, Result};

pub(in crate::wait) trait WaiterProcess {
    fn is_running(&self, pid: u32) -> Result<bool>;
    fn terminate(&self, pid: u32) -> Result<()>;
}

pub(super) struct SystemWaiterProcess;

impl WaiterProcess for SystemWaiterProcess {
    fn is_running(&self, pid: u32) -> Result<bool> {
        process_is_running(pid)
    }

    fn terminate(&self, pid: u32) -> Result<()> {
        terminate_process(pid)
    }
}

fn process_is_running(pid: u32) -> Result<bool> {
    let pid = pid_to_pid_t(pid)?;
    // SAFETY: signal 0 performs permission/existence checks without delivering a signal.
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Ok(true);
    }

    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::ESRCH => Ok(false),
        Some(code) if code == libc::EPERM => Ok(true),
        _ => Err(err).with_context(|| format!("failed to check whether pid {pid} is running")),
    }
}

fn terminate_process(pid: u32) -> Result<()> {
    let pid = pid_to_pid_t(pid)?;
    // SAFETY: SIGTERM is delivered to the registered waiter pid only.
    let result = unsafe { libc::kill(pid, libc::SIGTERM) };
    if result == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::ESRCH => Ok(()),
        _ => Err(err).with_context(|| format!("failed to send SIGTERM to pid {pid}")),
    }
}

fn pid_to_pid_t(pid: u32) -> Result<libc::pid_t> {
    libc::pid_t::try_from(pid)
        .with_context(|| format!("waiter pid {pid} does not fit platform pid_t"))
}
