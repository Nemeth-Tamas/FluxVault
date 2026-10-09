//! Bind all controller-created host processes to the Windows controller lifetime.
//!
//! Assign the controller BEFORE any host spawn. Windows then assigns descendants
//! at process creation, avoiding the spawn-then-assign race. The single unnamed,
//! non-inheritable job handle is deliberately retained until OS process teardown;
//! closing it earlier would also terminate this controller. No breakaway flags,
//! resource limits, named global objects or administrative privileges are used.
use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Output, Stdio},
};

#[cfg(windows)]
pub fn ensure() -> Result<(), String> {
    use std::sync::OnceLock;
    static JOB: OnceLock<Result<usize, String>> = OnceLock::new();
    JOB.get_or_init(create_controller_job)
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

#[cfg(windows)]
fn create_controller_job() -> Result<usize, String> {
    use windows::{
        Win32::{
            Foundation::CloseHandle,
            System::{
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                    SetInformationJobObject,
                },
                Threading::GetCurrentProcess,
            },
        },
        core::PCWSTR,
    };
    // SAFETY: unnamed job, default security attributes => non-inheritable handle.
    // The input structure is fully initialized and alive for the synchronous call.
    // GetCurrentProcess is a pseudo-handle, not owned/closed by this function.
    unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).map_err(|e| {
            format!("Cannot create Windows controller job; host launch refused: {e}")
        })?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        )
        .and_then(|_| AssignProcessToJobObject(job, GetCurrentProcess()));
        if let Err(e) = configured {
            // Assignment has not succeeded; the empty job can be safely closed.
            let _ = CloseHandle(job);
            return Err(format!(
                "Cannot bind Windows controller job; no unsupervised host launch allowed: {e}. Check launcher/job restrictions; use a normal PowerShell session."
            ));
        }
        // Retain exactly one process-owned handle. Never expose or duplicate it.
        // On normal exit, Ctrl+C or forced termination Windows closes it and kills
        // descendants; Rust static destructors/cleanup handlers are not needed.
        Ok(job.0 as usize)
    }
}

#[cfg(not(windows))]
pub fn ensure() -> Result<(), String> {
    // Other platforms retain their existing runners/timeouts. Do not claim the
    // Windows lifetime guarantee on platforms without this implementation.
    Ok(())
}

pub(crate) fn audit_mode() -> Option<String> {
    #[cfg(windows)]
    {
        ensure()
            .ok()
            .map(|_| "windows_controller_and_operation_jobs".into())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub(crate) struct ManagedChild {
    child: Child,
    #[cfg(windows)]
    job: Option<WinHandle>,
}
impl std::ops::Deref for ManagedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl std::ops::DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl ManagedChild {
    fn close_job(&mut self) {
        #[cfg(windows)]
        {
            self.job.take();
        }
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.finish_job()?;
        }
        Ok(status)
    }
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.finish_job()?;
        Ok(status)
    }
    fn finish_job(&mut self) -> std::io::Result<()> {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            use windows::Win32::System::JobObjects::{
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
                QueryInformationJobObject, TerminateJobObject,
            };
            // SAFETY: exclusively owned operation job, never the controller job.
            unsafe {
                TerminateJobObject(job.0, 1).map_err(std::io::Error::other)?;
            }
            let deadline = std::time::Instant::now();
            loop {
                let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
                unsafe {
                    QueryInformationJobObject(
                        Some(job.0),
                        JobObjectBasicAccountingInformation,
                        &mut info as *mut _ as *mut _,
                        std::mem::size_of_val(&info) as u32,
                        None,
                    )
                    .map_err(std::io::Error::other)?;
                }
                if info.ActiveProcesses == 0 {
                    break;
                }
                if deadline.elapsed() >= std::time::Duration::from_secs(5) {
                    return Err(std::io::Error::other(
                        "Operation job termination not confirmed within 5s",
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        self.close_job();
        Ok(())
    }
    pub(crate) fn terminate_tree(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.finish_job().map_err(|e| e.to_string())
        }
        #[cfg(not(windows))]
        {
            crate::external_tools::terminate_process_tree(&mut self.child)
        }
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.close_job();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(windows)]
struct WinHandle(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for WinHandle {
    fn drop(&mut self) {
        // SAFETY: a single owned, non-inheritable valid Win32 handle.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
// SAFETY: Win32 kernel handles may be owned/closed on a different thread. The
// operation guard is exclusively owned; it never duplicates or exposes its job.
#[cfg(windows)]
unsafe impl Send for WinHandle {}

pub(crate) fn spawn(command: &mut Command) -> std::io::Result<ManagedChild> {
    crate::cancellation::check().map_err(std::io::Error::other)?;
    ensure().map_err(std::io::Error::other)?;
    #[cfg(windows)]
    {
        spawn_windows(command).map_err(std::io::Error::other)
    }
    #[cfg(not(windows))]
    {
        Ok(ManagedChild {
            child: command.spawn()?,
        })
    }
}

pub(crate) fn output(command: &mut Command) -> std::io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn(command)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("Missing stdout pipe"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("Missing stderr pipe"))?;
    let out = std::thread::spawn(move || {
        let mut bytes = vec![];
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let err = std::thread::spawn(move || {
        let mut bytes = vec![];
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = (|| {
        loop {
            if let Some(status) = child.try_wait()? {
                break Ok(status);
            }
            if crate::cancellation::requested() {
                child.terminate_tree().map_err(std::io::Error::other)?;
                break child.wait();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    })();
    // Close the operation job before joining BOTH pipes, even on wait/kill
    // failure. Do not leave detached readers behind when reporting STOPPED.
    drop(child);
    let stdout = out
        .join()
        .map_err(|_| std::io::Error::other("stdout reader panicked"));
    let stderr = err
        .join()
        .map_err(|_| std::io::Error::other("stderr reader panicked"));
    Ok(Output {
        status: status?,
        stdout: stdout??,
        stderr: stderr??,
    })
}

#[cfg(windows)]
fn spawn_windows(command: &mut Command) -> Result<ManagedChild, String> {
    spawn_windows_with_resume(command, resume_initial_thread)
}

#[cfg(windows)]
fn spawn_windows_with_resume(
    command: &mut Command,
    resume: impl FnOnce(u32) -> Result<(), String>,
) -> Result<ManagedChild, String> {
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    use windows::{
        Win32::{
            Foundation::HANDLE,
            System::{
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                    SetInformationJobObject,
                },
                Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED},
            },
        },
        core::PCWSTR,
    };
    // SAFETY: initialized structures and exclusively owned handles; no breakaway.
    let job = unsafe {
        let job = WinHandle(CreateJobObjectW(None, PCWSTR::null()).map_err(|e| e.to_string())?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        )
        .map_err(|e| e.to_string())?;
        job
    };
    // The outer controller job is already installed: even termination between
    // this suspended creation and inner assignment cannot orphan the host.
    command.creation_flags((CREATE_SUSPENDED | CREATE_NO_WINDOW).0);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    // SAFETY: borrowed live child handle, valid job with no resource/UI limits.
    if let Err(e) = unsafe { AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle())) } {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "Operation job assignment failed before host resume: {e}"
        ));
    }
    let managed = ManagedChild {
        child,
        job: Some(job),
    };
    resume(managed.child.id())?; // guard kills suspended host on error
    Ok(managed)
}

#[cfg(windows)]
fn resume_initial_thread(pid: u32) -> Result<(), String> {
    use windows::Win32::System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    };
    // Rust's main_thread_handle API is nightly-only. The newly created process
    // is suspended, so require exactly one snapshot thread belonging to its PID;
    // ambiguous/missing enumeration is a refusal, never resume arbitrary threads.
    unsafe {
        let snapshot =
            WinHandle(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(|e| e.to_string())?);
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        Thread32First(snapshot.0, &mut entry).map_err(|e| e.to_string())?;
        let mut ids = vec![];
        loop {
            if entry.th32OwnerProcessID == pid {
                ids.push(entry.th32ThreadID);
            }
            match Thread32Next(snapshot.0, &mut entry) {
                Ok(()) => {}
                Err(e) if e.code().0 as u32 == 0x80070012 => break, // ERROR_NO_MORE_FILES
                Err(e) => return Err(format!("Suspended host thread enumeration failed: {e}")),
            }
        }
        if ids.len() != 1 {
            return Err(format!(
                "Suspended host has {} candidate threads; resume refused",
                ids.len()
            ));
        }
        let thread =
            WinHandle(OpenThread(THREAD_SUSPEND_RESUME, false, ids[0]).map_err(|e| e.to_string())?);
        if ResumeThread(thread.0) != 1 {
            return Err("Suspended host thread resume failed/inconsistent".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn failed_resume_kills_the_still_suspended_host_before_any_execution() {
        use windows::Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        super::ensure().unwrap();
        let mut observed: Option<super::WinHandle> = None;
        let mut cmd = std::process::Command::new(
            std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/cmd.exe"),
        );
        cmd.args(["/C", "exit", "0"]);
        let outcome = super::spawn_windows_with_resume(&mut cmd, |pid| {
            let h: HANDLE = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid).unwrap() };
            assert_eq!(unsafe { WaitForSingleObject(h, 0) }, WAIT_TIMEOUT);
            observed = Some(super::WinHandle(h));
            Err("injected resume refusal".into())
        });
        assert_eq!(outcome.err().unwrap(), "injected resume refusal");
        assert_eq!(
            unsafe { WaitForSingleObject(observed.unwrap().0, 5000) },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn missing_executable_fails_without_falling_back_to_an_unmanaged_launch() {
        let path = std::env::temp_dir().join(format!(
            "fv-absent-host-{}-{}.exe",
            std::process::id(),
            crate::external_tools::current_unix_ms()
        ));
        assert!(!path.exists());
        assert!(super::spawn(&mut std::process::Command::new(path)).is_err());
    }

    #[test]
    fn concurrent_initialization_is_idempotent_and_reports_only_supported_guarantees() {
        let workers = (0..16)
            .map(|_| std::thread::spawn(super::ensure))
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        #[cfg(windows)]
        assert_eq!(
            super::audit_mode().as_deref(),
            Some("windows_controller_and_operation_jobs")
        );
        #[cfg(not(windows))]
        assert_eq!(super::audit_mode(), None);
    }
}
