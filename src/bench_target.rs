use crate::connection::{
    Connection, ConnectionError, IncomingMessage, MessageError, OutgoingMessage,
};
use std::ffi::OsString;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};

#[derive(Debug)]
pub enum TargetError {
    IoError(String, std::io::Error),
    TargetFailed(String, ExitStatus),
    MessageError(String, MessageError),
    ConnectionError(String, ConnectionError),
}
impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetError::TargetFailed(target_name, exit_status) => write!(
                f,
                "Benchmark target '{}' returned an error ({}).",
                target_name, exit_status
            ),
            TargetError::IoError(target_name, io_error) => write!(
                f,
                "Unexpected IO Error while running benchmark target '{}':\n{}",
                target_name, io_error
            ),
            TargetError::MessageError(target_name, message_error) => write!(
                f,
                "Unexpected error communicating with benchmark target '{}':\n{}",
                target_name, message_error
            ),
            TargetError::ConnectionError(target_name, connection_error) => write!(
                f,
                "Unexpected error connecting to benchmark target '{}':\n{}",
                target_name, connection_error
            ),
        }
    }
}
impl std::error::Error for TargetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TargetError::TargetFailed(_, _) => None,
            TargetError::IoError(_, io_error) => Some(io_error),
            TargetError::MessageError(_, message_error) => Some(message_error),
            TargetError::ConnectionError(_, connection_error) => Some(connection_error),
        }
    }
}

/// Structure representing a compiled benchmark executable.
#[derive(Debug)]
pub struct BenchTarget {
    pub name: String,
    pub executable: PathBuf,
}
impl BenchTarget {
    pub fn execute(
        &self,
        criterion_home: &PathBuf,
        additional_args: &[OsString],
    ) -> Result<(), TargetError> {
        let listener = TcpListener::bind("localhost:0")
            .map_err(|err| TargetError::IoError(self.name.clone(), err))?;
        listener
            .set_nonblocking(true)
            .map_err(|err| TargetError::IoError(self.name.clone(), err))?;

        let addr = listener
            .local_addr()
            .map_err(|err| TargetError::IoError(self.name.clone(), err))?;
        let port = addr.port();

        let mut command = Command::new(&self.executable);
        command
            .arg("--bench")
            .args(additional_args)
            .env("CRITERION_HOME", criterion_home)
            .env("CARGO_CRITERION_PORT", &port.to_string())
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit());

        println!("{:?}", command);

        let mut child = command
            .spawn()
            .map_err(|err| TargetError::IoError(self.name.clone(), err))?;

        loop {
            match listener.accept() {
                Ok((socket, _)) => {
                    let conn = Connection::new(socket)
                        .map_err(|err| TargetError::ConnectionError(self.name.clone(), err))?;
                    return self.communicate(&mut child, conn);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection yet, try again in a bit.
                }
                Err(e) => {
                    println!("Failed to accept connection");
                    return Err(TargetError::IoError(self.name.clone(), e));
                }
            };

            match child.try_wait() {
                Err(e) => {
                    println!("Failed to poll child process");
                    return Err(TargetError::IoError(self.name.clone(), e));
                }
                Ok(Some(exit_status)) => {
                    if exit_status.success() {
                        println!("Child exited successfully");
                        return Ok(());
                    } else {
                        println!("Child terminated");
                        return Err(TargetError::TargetFailed(self.name.clone(), exit_status));
                    }
                }
                Ok(None) => (), // Child still running, keep trying.
            };

            // Wait a bit then poll again.
            std::thread::yield_now();
        }
    }

    fn communicate(&self, child: &mut Child, mut conn: Connection) -> Result<(), TargetError> {
        loop {
            let message = conn
                .recv()
                .map_err(|err| TargetError::MessageError(self.name.clone(), err))?;
            if message.is_none() {
                return Ok(());
            }
            let message = message.unwrap();
            match message {
                IncomingMessage::BeginningBenchmarkGroup { group } => {
                    println!("Beginning benchmark group {}", group);
                }
                IncomingMessage::FinishedBenchmarkGroup { group } => {
                    println!("Finished benchmark group {}", group);
                }
                IncomingMessage::BeginningBenchmark { id } => {
                    println!("Beginning benchmark {:?}", id);
                    conn.send(&OutgoingMessage::RunBenchmark)
                        .map_err(|err| TargetError::MessageError(self.name.clone(), err))?;
                }
                IncomingMessage::SkippingBenchmark { id } => {
                    println!("Skipping benchmark {:?}", id)
                }
                IncomingMessage::Warmup { id, nanos } => {
                    println!("Warming up benchmark {:?} for {} nanos", id, nanos)
                }
                IncomingMessage::MeasurementStart {
                    id,
                    sample_count,
                    estimate_ns,
                    iter_count,
                    added_runner,
                } => {
                    println!("Measuring benchmark {:?} samples: {}, estimated time: {}ns, iterations: {}, {:?}", id, sample_count, estimate_ns, iter_count, added_runner);
                }
                IncomingMessage::MeasurementComplete {
                    id,
                    iters: _,
                    times: _,
                } => {
                    println!("Measurement of benchmark {:?} complete", id);
                }
            }

            match child.try_wait() {
                Err(e) => {
                    println!("Failed to poll Criterion.rs child process");
                    return Err(TargetError::IoError(self.name.clone(), e));
                }
                Ok(Some(exit_status)) => {
                    if exit_status.success() {
                        println!("Criterion.rs child exited successfully");
                        return Ok(());
                    } else {
                        println!("Criterion.rs child terminated unsuccessfully");
                        return Err(TargetError::TargetFailed(self.name.clone(), exit_status));
                    }
                }
                Ok(None) => continue,
            };
        }
    }
}
