// SPDX-License-Identifier: AGPL-3.0-or-later
//! Child processes (ffmpeg, behaviors).

use std::process::{Child, Command};

/// A command that doesn't flash a console window on Windows.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Ties the lifetime of `child` to ours, so it doesn't survive a crash of Open Anima.
pub fn adopt(child: &Child) {
    #[cfg(windows)]
    windows::adopt(child);
    #[cfg(not(windows))]
    let _ = child;
}

#[cfg(windows)]
mod windows {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr::null;
    use std::sync::OnceLock;

    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// The job handle is never closed: the system closes it when we exit, which kills the
    /// processes assigned to it.
    struct Job(isize);

    fn job() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| unsafe {
            let handle = CreateJobObjectW(null(), null());
            if handle.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            Some(Job(handle as isize))
        })
        .as_ref()
    }

    pub fn adopt(child: &Child) {
        if let Some(job) = job() {
            unsafe { AssignProcessToJobObject(job.0 as _, child.as_raw_handle()) };
        }
    }
}
