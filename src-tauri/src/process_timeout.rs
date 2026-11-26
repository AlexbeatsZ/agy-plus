//! Bounded subprocess helpers for registry/keychain probes that must not hang UI.

use std::io::{self, Read, Write};
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Run `command` and wait up to `timeout`. On timeout, kill the child and return
/// `io::ErrorKind::TimedOut`.
///
/// The wall-clock bound covers both process lifetime and stdout/stderr drain.
/// If a descendant holds an inherited pipe after the parent exits, join is aborted
/// when `timeout` elapses (reader threads are detached).
pub fn output_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    output_with_optional_input_timeout(command, None, timeout)
}

/// Like `output_with_timeout`, but send input to stdin and close it on completion.
/// The input writer shares the same deadline and cannot block process monitoring.
#[cfg(target_os = "linux")]
pub fn output_with_input_timeout(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
) -> io::Result<Output> {
    output_with_optional_input_timeout(command, Some(input.to_vec()), timeout)
}

fn output_with_optional_input_timeout(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> io::Result<Output> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let child = command.spawn();
    let mut child = child?;
    let stdin_writer = input.and_then(|input| {
        child
            .stdin
            .take()
            .map(|mut pipe| std::thread::spawn(move || pipe.write_all(&input)))
    });
    let stdout_reader = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).map(|_| output)
        })
    });
    let stderr_reader = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            pipe.read_to_end(&mut output).map(|_| output)
        })
    });
    let deadline = Instant::now() + timeout;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Do not join readers on the kill path: a surviving descendant may
                    // still hold an inherited pipe, and the timeout path must stay bounded.
                    drop(stdout_reader);
                    drop(stderr_reader);
                    drop(stdin_writer);
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("process timed out after {:?}", timeout),
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stdout_reader);
                drop(stderr_reader);
                drop(stdin_writer);
                return Err(error);
            }
        }
    };

    join_with_deadline(stdin_writer, deadline)?;
    let stdout = join_with_deadline(stdout_reader, deadline)?;
    let stderr = join_with_deadline(stderr_reader, deadline)?;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn join_with_deadline<T: Default>(
    reader: Option<JoinHandle<io::Result<T>>>,
    deadline: Instant,
) -> io::Result<T> {
    let Some(handle) = reader else {
        return Ok(T::default());
    };
    loop {
        if handle.is_finished() {
            return handle
                .join()
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "process pipe worker panicked"))?
                .map_err(|error| error);
        }
        if Instant::now() >= deadline {
            // Detach the reader thread; it will exit when the pipe is eventually closed.
            drop(handle);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process pipe drain timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
