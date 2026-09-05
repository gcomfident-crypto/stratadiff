use std::{
    fmt,
    io::Read,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};

const PROCESS_PIPE_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROCESS_SIGNAL_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct CapturedOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) struct SignalState {
    received: Arc<AtomicUsize>,
    #[cfg(unix)]
    registrations: Vec<signal_hook::SigId>,
}

impl SignalState {
    pub(crate) fn register() -> Result<Self> {
        let received = Arc::new(AtomicUsize::new(0));
        #[cfg(unix)]
        let registrations = {
            use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};

            let mut registrations = Vec::new();
            for signal in [SIGINT, SIGTERM, SIGHUP] {
                let signal_value = usize::try_from(signal)
                    .context("shutdown signal number cannot be represented as usize")?;
                match signal_hook::flag::register_usize(signal, Arc::clone(&received), signal_value)
                {
                    Ok(registration) => registrations.push(registration),
                    Err(error) => {
                        for registration in registrations {
                            signal_hook::low_level::unregister(registration);
                        }
                        return Err(error)
                            .with_context(|| format!("failed to register signal {signal}"));
                    }
                }
            }
            registrations
        };

        Ok(Self {
            received,
            #[cfg(unix)]
            registrations,
        })
    }

    pub(crate) fn received(&self) -> Option<i32> {
        let signal = self.received.load(Ordering::Acquire);
        (signal != 0).then(|| {
            i32::try_from(signal).expect("registered signal numbers are representable as i32")
        })
    }

    pub(crate) fn check(&self) -> Result<()> {
        match self.received() {
            Some(signal) => Err(Interrupted::new(signal).into()),
            None => Ok(()),
        }
    }
}

impl Drop for SignalState {
    fn drop(&mut self) {
        #[cfg(unix)]
        for registration in self.registrations.drain(..) {
            signal_hook::low_level::unregister(registration);
        }
    }
}

#[derive(Debug)]
pub(crate) struct Interrupted {
    signal: i32,
}

impl Interrupted {
    fn new(signal: i32) -> Self {
        Self { signal }
    }

    pub(crate) fn signal(&self) -> i32 {
        self.signal
    }
}

impl fmt::Display for Interrupted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "interrupted by signal {}", self.signal)
    }
}

impl std::error::Error for Interrupted {}

pub(crate) fn run_bounded_process(
    command: &mut Command,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    label: &str,
    signals: Option<&SignalState>,
) -> Result<CapturedOutput> {
    run_bounded_process_recording_pid(
        command,
        stdout_limit,
        stderr_limit,
        timeout,
        label,
        signals,
        &mut None,
    )
}

pub(crate) fn run_bounded_process_recording_pid(
    command: &mut Command,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Duration,
    label: &str,
    signals: Option<&SignalState>,
    spawned_pid: &mut Option<u32>,
) -> Result<CapturedOutput> {
    if let Some(signal) = signals.and_then(SignalState::received) {
        return Err(Interrupted::new(signal).into());
    }

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(command);
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {label}"))?;
    *spawned_pid = Some(child.id());
    let stdout = child
        .stdout
        .take()
        .with_context(|| format!("failed to capture {label} stdout"))?;
    let stderr = child
        .stderr
        .take()
        .with_context(|| format!("failed to capture {label} stderr"))?;
    let overflow = Arc::new(AtomicU8::new(0));
    let stdout_overflow = Arc::clone(&overflow);
    let stderr_overflow = Arc::clone(&overflow);
    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = stdout_sender.send(read_capped_stream(
            stdout,
            stdout_limit,
            &stdout_overflow,
            1,
        ));
    });
    thread::spawn(move || {
        let _ = stderr_sender.send(read_capped_stream(
            stderr,
            stderr_limit,
            &stderr_overflow,
            2,
        ));
    });
    let started = Instant::now();
    let mut timed_out = false;
    let mut interrupted = None;
    let status = loop {
        if let Some(signal) = signals.and_then(SignalState::received) {
            interrupted = Some(signal);
            break forward_signal_and_reap(
                &mut child,
                signal,
                PROCESS_SIGNAL_GRACE_PERIOD,
                label,
                true,
            )?;
        }
        if overflow.load(Ordering::Acquire) != 0 {
            break terminate_child(&mut child, label)?;
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
        {
            terminate_remaining_process_group(&child, label)?;
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            break terminate_child(&mut child, label)?;
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    };
    let pipe_deadline = Instant::now() + PROCESS_PIPE_CLOSE_TIMEOUT;
    let (stdout, stdout_exceeded) =
        receive_bounded_stream(&stdout_receiver, pipe_deadline, &format!("{label} stdout"))?;
    let (stderr, stderr_exceeded) =
        receive_bounded_stream(&stderr_receiver, pipe_deadline, &format!("{label} stderr"))?;
    ensure!(
        !stdout_exceeded,
        "{label} stdout bytes limit exceeded: limit {stdout_limit}"
    );
    ensure!(
        !stderr_exceeded,
        "{label} stderr bytes limit exceeded: limit {stderr_limit}"
    );
    if let Some(signal) = interrupted {
        return Err(Interrupted::new(signal).into());
    }
    ensure!(
        !timed_out,
        "{label} timed out after {} milliseconds",
        timeout.as_millis()
    );
    Ok(CapturedOutput {
        status,
        stdout,
        stderr,
    })
}

pub(crate) fn run_short_critical_process(
    command: &mut Command,
    timeout: Duration,
    label: &str,
    spawned_pid: &mut Option<u32>,
) -> Result<ExitStatus> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_process_group(command);
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {label}"))?;
    *spawned_pid = Some(child.id());
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
        {
            terminate_remaining_process_group(&child, label)?;
            return Ok(status);
        }
        if started.elapsed() >= timeout {
            let _ = terminate_child(&mut child, label)?;
            bail!(
                "{label} timed out after {} milliseconds",
                timeout.as_millis()
            );
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

pub(crate) fn run_inherited_process(
    command: &mut Command,
    signals: &SignalState,
    shutdown_grace: Duration,
    label: &str,
) -> Result<ExitStatus> {
    if let Some(signal) = signals.received() {
        return Err(Interrupted::new(signal).into());
    }

    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    configure_process_group(command);
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {label}"))?;
    loop {
        if let Some(signal) = signals.received() {
            let _ = forward_signal_and_reap(&mut child, signal, shutdown_grace, label, false)?;
            return Err(Interrupted::new(signal).into());
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label}"))?
        {
            return Ok(status);
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

fn configure_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

fn receive_bounded_stream(
    receiver: &mpsc::Receiver<std::io::Result<(Vec<u8>, bool)>>,
    deadline: Instant,
    label: &str,
) -> Result<(Vec<u8>, bool)> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    receiver
        .recv_timeout(remaining)
        .with_context(|| format!("{label} pipe did not close after the child exited"))?
        .with_context(|| format!("failed to read {label}"))
}

fn forward_signal_and_reap(
    child: &mut std::process::Child,
    signal: i32,
    shutdown_grace: Duration,
    label: &str,
    terminate_descendants_after_exit: bool,
) -> Result<ExitStatus> {
    forward_signal_to_process_group(child, signal, label)?;
    let deadline = Instant::now() + shutdown_grace;
    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait for {label} after forwarding signal"))?
        {
            if terminate_descendants_after_exit {
                terminate_remaining_process_group(child, label)?;
            }
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return terminate_child(child, label);
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

fn terminate_child(
    child: &mut std::process::Child,
    label: &str,
) -> Result<std::process::ExitStatus> {
    terminate_remaining_process_group(child, label)?;
    match child.kill() {
        Ok(()) => child
            .wait()
            .with_context(|| format!("failed to reap {label} after killing it")),
        Err(kill_error) => match child
            .try_wait()
            .with_context(|| format!("failed to inspect {label} after kill failed"))?
        {
            Some(status) => Ok(status),
            None => Err(kill_error).with_context(|| format!("failed to kill {label}")),
        },
    }
}

#[cfg(unix)]
fn forward_signal_to_process_group(
    child: &mut std::process::Child,
    signal: i32,
    label: &str,
) -> Result<()> {
    let process_group = i32::try_from(child.id()).context("child process ID exceeds i32")?;
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error).with_context(|| format!("failed to forward signal to {label}"))
    }
}

#[cfg(not(unix))]
fn forward_signal_to_process_group(
    child: &mut std::process::Child,
    _signal: i32,
    label: &str,
) -> Result<()> {
    child
        .kill()
        .with_context(|| format!("failed to stop {label}"))
}

#[cfg(unix)]
fn terminate_remaining_process_group(child: &std::process::Child, label: &str) -> Result<()> {
    let process_group = i32::try_from(child.id()).context("child process ID exceeds i32")?;
    let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error).with_context(|| format!("failed to terminate remaining {label} processes"))
    }
}

#[cfg(not(unix))]
fn terminate_remaining_process_group(_child: &std::process::Child, _label: &str) -> Result<()> {
    Ok(())
}

fn read_capped_stream(
    mut reader: impl Read,
    limit: usize,
    overflow: &AtomicU8,
    overflow_bit: u8,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let available = limit.saturating_sub(retained.len());
        let keep = available.min(read);
        retained.extend_from_slice(&buffer[..keep]);
        exceeded |= keep < read;
        if exceeded {
            overflow.fetch_or(overflow_bit, Ordering::Release);
        }
    }
    Ok((retained, exceeded))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        process::Command,
        sync::{Arc, atomic::Ordering},
        thread,
        time::{Duration, Instant},
    };

    use super::{
        Interrupted, SignalState, run_bounded_process, run_bounded_process_recording_pid,
        run_inherited_process, run_short_critical_process,
    };

    #[cfg(unix)]
    #[test]
    fn short_critical_process_has_a_hard_deadline() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let started = Instant::now();
        let mut spawned_pid = None;

        let error = run_short_critical_process(
            &mut command,
            Duration::from_millis(100),
            "critical timeout fixture",
            &mut spawned_pid,
        )
        .unwrap_err();

        assert!(spawned_pid.is_some());
        assert!(
            error
                .to_string()
                .contains("timed out after 100 milliseconds")
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_terminates_after_its_deadline() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let started = Instant::now();

        let error = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_millis(100),
            "timeout fixture",
            None,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("timed out after 100 milliseconds")
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_closes_pipes_held_by_descendants() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & exit 0"]);
        let started = Instant::now();

        let output = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_secs(5),
            "descendant fixture",
            None,
        )
        .unwrap();

        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_process_does_not_wait_forever_for_an_escaped_descendant_pipe() {
        let directory = tempfile::tempdir().unwrap();
        let pid_path = directory.path().join("escaped.pid");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "setsid sh -c 'printf \"%s\" \"$$\" > \"$ESCAPED_PID_PATH\"; sleep 30' & while [ ! -s \"$ESCAPED_PID_PATH\" ]; do :; done",
            ])
            .env("ESCAPED_PID_PATH", &pid_path);
        let started = Instant::now();

        let error = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_secs(5),
            "escaped descendant fixture",
            None,
        )
        .unwrap_err();
        let escaped_pid = fs::read_to_string(pid_path)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        unsafe {
            libc::kill(-escaped_pid, libc::SIGKILL);
        }

        assert!(error.to_string().contains("pipe did not close"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_stops_when_output_exceeds_its_limit() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do printf 0123456789; done"]);
        let started = Instant::now();

        let error = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_secs(5),
            "output fixture",
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("stdout bytes limit exceeded"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_records_the_spawned_process_id() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf %s $$"]);
        let mut spawned_pid = None;

        let output = run_bounded_process_recording_pid(
            &mut command,
            1024,
            1024,
            Duration::from_secs(5),
            "process ID fixture",
            None,
            &mut spawned_pid,
        )
        .unwrap();

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            spawned_pid.unwrap().to_string()
        );
    }

    #[cfg(unix)]
    #[test]
    fn bounded_process_forwards_the_recorded_signal_and_reaps() {
        let directory = tempfile::tempdir().unwrap();
        let signal_path = directory.path().join("signal.txt");
        let state = SignalState::register().unwrap();
        let received = Arc::clone(&state.received);
        let signal_path_for_thread = signal_path.clone();
        let trigger = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !signal_path_for_thread.with_extension("ready").exists() {
                assert!(Instant::now() < deadline, "fixture did not become ready");
                thread::sleep(Duration::from_millis(10));
            }
            received.store(libc::SIGTERM as usize, Ordering::Release);
        });
        let ready_path = signal_path.with_extension("ready");
        let script = format!(
            "trap 'printf TERM > \"{}\"; exit 0' TERM; : > \"{}\"; while :; do :; done",
            signal_path.display(),
            ready_path.display()
        );
        let mut command = Command::new("sh");
        command.args(["-c", &script]);

        let error = run_bounded_process(
            &mut command,
            1024,
            1024,
            Duration::from_secs(10),
            "signal fixture",
            Some(&state),
        )
        .unwrap_err();
        trigger.join().unwrap();

        assert_eq!(
            error.downcast_ref::<Interrupted>().unwrap().signal(),
            libc::SIGTERM
        );
        assert_eq!(fs::read_to_string(signal_path).unwrap(), "TERM");
    }

    #[cfg(unix)]
    #[test]
    fn inherited_process_forwards_the_recorded_signal_and_reaps() {
        let directory = tempfile::tempdir().unwrap();
        let signal_path = directory.path().join("signal.txt");
        let ready_path = directory.path().join("ready");
        let state = SignalState::register().unwrap();
        let received = Arc::clone(&state.received);
        let ready_path_for_thread = ready_path.clone();
        let trigger = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !ready_path_for_thread.exists() {
                assert!(Instant::now() < deadline, "fixture did not become ready");
                thread::sleep(Duration::from_millis(10));
            }
            received.store(libc::SIGHUP as usize, Ordering::Release);
        });
        let script = format!(
            "trap 'printf HUP > \"{}\"; exit 0' HUP; : > \"{}\"; while :; do :; done",
            signal_path.display(),
            ready_path.display()
        );
        let mut command = Command::new("sh");
        command.args(["-c", &script]);

        let error = run_inherited_process(
            &mut command,
            &state,
            Duration::from_secs(2),
            "inherited signal fixture",
        )
        .unwrap_err();
        trigger.join().unwrap();

        assert_eq!(
            error.downcast_ref::<Interrupted>().unwrap().signal(),
            libc::SIGHUP
        );
        assert_eq!(fs::read_to_string(signal_path).unwrap(), "HUP");
    }
}
