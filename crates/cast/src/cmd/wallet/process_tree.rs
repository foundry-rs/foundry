use std::{
    io,
    process::{Command, ExitStatus},
    time::Duration,
};

use tokio::process::{Child, Command as TokioCommand};

const TERMINATION_GRACE: Duration = Duration::from_millis(250);

pub(super) struct ManagedChild {
    child: Child,
    group: PlatformProcessGroup,
    waited: bool,
}

impl ManagedChild {
    pub(super) fn spawn(command: Command) -> io::Result<Self> {
        let mut command = TokioCommand::from(command);
        let group = PlatformProcessGroup::configure(&mut command)?;
        let child = command.kill_on_drop(true).spawn()?;
        let group = group.attach(&child)?;
        Ok(Self { child, group, waited: false })
    }

    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.waited = true;
        Ok(status)
    }

    pub(super) async fn terminate_tree(&mut self) -> io::Result<()> {
        if self.group.terminate()? {
            tokio::time::sleep(TERMINATION_GRACE).await;
            self.group.kill()?;
        }

        if self.waited {
            return Ok(());
        }

        match tokio::time::timeout(TERMINATION_GRACE, self.child.wait()).await {
            Ok(result) => {
                self.waited = true;
                result.map(|_| ())
            }
            Err(_) => {
                self.child.start_kill()?;
                self.child.wait().await?;
                self.waited = true;
                Ok(())
            }
        }
    }
}

#[cfg(unix)]
struct PlatformProcessGroup {
    pgid: Option<libc::pid_t>,
}

#[cfg(unix)]
impl PlatformProcessGroup {
    fn configure(command: &mut TokioCommand) -> io::Result<Self> {
        command.process_group(0);
        Ok(Self { pgid: None })
    }

    fn attach(mut self, child: &Child) -> io::Result<Self> {
        self.pgid = child.id().map(|id| id as libc::pid_t);
        Ok(self)
    }

    fn terminate(&mut self) -> io::Result<bool> {
        let Some(pgid) = self.pgid else {
            return Ok(false);
        };
        signal_process_group(pgid, libc::SIGTERM)
    }

    fn kill(&mut self) -> io::Result<()> {
        let Some(pgid) = self.pgid.take() else {
            return Ok(());
        };
        signal_process_group(pgid, libc::SIGKILL).map(|_| ())
    }
}

#[cfg(unix)]
fn signal_process_group(pgid: libc::pid_t, signal: libc::c_int) -> io::Result<bool> {
    // SAFETY: negative pid targets the process group created for the child process.
    let rc = unsafe { libc::kill(-pgid, signal) };
    if rc == 0 {
        Ok(true)
    } else {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) { Ok(false) } else { Err(err) }
    }
}

#[cfg(windows)]
struct PlatformProcessGroup {
    job: Option<WindowsJob>,
}

#[cfg(windows)]
impl PlatformProcessGroup {
    fn configure(_command: &mut TokioCommand) -> io::Result<Self> {
        Ok(Self { job: Some(WindowsJob::new()?) })
    }

    fn attach(self, child: &Child) -> io::Result<Self> {
        if let Some(job) = &self.job {
            let handle = child.raw_handle().ok_or_else(|| {
                io::Error::new(io::ErrorKind::Other, "session child exited before job assignment")
            })?;
            job.assign_process(handle)?;
        }
        Ok(self)
    }

    fn terminate(&mut self) -> io::Result<bool> {
        if let Some(job) = self.job.take() {
            job.terminate()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
struct WindowsJob {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WindowsJob {
    fn new() -> io::Result<Self> {
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            System::JobObjects::{
                CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
        };

        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
            return Err(err);
        }

        Ok(Self { handle })
    }

    fn assign_process(&self, process: std::os::windows::io::RawHandle) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let ok = unsafe { AssignProcessToJobObject(self.handle, process.cast()) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn terminate(self) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        let ok = unsafe { TerminateJobObject(self.handle, 1) };
        let err = if ok == 0 { Some(io::Error::last_os_error()) } else { None };
        drop(self);
        match err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(not(any(unix, windows)))]
struct PlatformProcessGroup;

#[cfg(not(any(unix, windows)))]
impl PlatformProcessGroup {
    fn configure(_command: &mut TokioCommand) -> io::Result<Self> {
        Ok(Self)
    }

    fn attach(self, _child: &Child) -> io::Result<Self> {
        Ok(self)
    }

    fn terminate(&mut self) -> io::Result<bool> {
        Ok(false)
    }

    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn cleanup_terminates_background_grandchild() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let tmp = tempfile::tempdir().unwrap();
            let marker = tmp.path().join("session-child-leaked");
            let mut command = Command::new("sh");
            command.args([
                "-c",
                "(sleep 1; touch \"$1\") &",
                "session-child",
                &marker.to_string_lossy(),
            ]);

            let mut child = ManagedChild::spawn(command).unwrap();
            child.wait().await.unwrap();
            child.terminate_tree().await.unwrap();
            tokio::time::sleep(Duration::from_millis(1200)).await;

            assert!(!marker.exists(), "background grandchild escaped session cleanup");
        });
    }
}
