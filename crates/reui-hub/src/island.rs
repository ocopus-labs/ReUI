//! Launches the island surface and restarts it if it crashes. The island is placed in a
//! kill-on-close job, so it can't outlive the hub even if the hub itself crashes.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;
use tokio::time::Instant;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::*;

const MAX_RESTARTS: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(60);

struct KillOnCloseJob(HANDLE);

// The handle is only used to assign processes and is closed on drop.
unsafe impl Send for KillOnCloseJob {}

impl KillOnCloseJob {
    fn new() -> Result<Self> {
        unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null())?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            Ok(Self(job))
        }
    }

    fn assign(&self, process: *mut c_void) -> Result<()> {
        unsafe { Ok(AssignProcessToJobObject(self.0, HANDLE(process))?) }
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        unsafe { _ = CloseHandle(self.0) };
    }
}

/// Returns when the user quits the island (clean exit), or errors if it keeps crashing.
pub async fn supervise() -> Result<()> {
    let exe = std::env::current_exe()?.with_file_name("reui-island.exe");
    let job = KillOnCloseJob::new()?;
    let mut crashes = VecDeque::new();
    loop {
        let mut child = Command::new(&exe).kill_on_drop(true).spawn().with_context(|| format!("can't start {}", exe.display()))?;
        if let Some(handle) = child.raw_handle() {
            job.assign(handle)?;
        }
        let status = child.wait().await?;
        if status.success() {
            println!("island: quit by the user");
            return Ok(());
        }
        let now = Instant::now();
        crashes.push_back(now);
        while crashes.front().is_some_and(|t| now - *t > RESTART_WINDOW) {
            crashes.pop_front();
        }
        if crashes.len() > MAX_RESTARTS {
            bail!("island crashed {} times in a minute; giving up", crashes.len());
        }
        eprintln!("island: exited with {status}; restarting");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
