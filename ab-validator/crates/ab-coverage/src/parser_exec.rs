use std::{
    ffi::OsStr,
    fmt, fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

/// Run a parser executable as `--mode aat` with `source` on stdin and decode its
/// AAT, whose nesting can exceed serde_json's default recursion limit.
///
/// # Errors
///
/// Returns an error when the parser cannot be spawned, exits unsuccessfully, or
/// writes output that is not JSON.
pub fn run_parser(parser: &Path, source: &[u8]) -> Result<Value> {
    let mut child = Command::new(parser)
        .args(["--mode", "aat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", parser.display()))?;
    let mut stdin = child.stdin.take().context("parser stdin")?;
    let input = source.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().context("wait for parser")?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("parser stdin writer panicked"))?
        .context("write parser stdin")?;
    if !output.status.success() {
        bail!(
            "parser exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut deserializer = serde_json::Deserializer::from_slice(&output.stdout);
    deserializer.disable_recursion_limit();
    let deserializer = serde_stacker::Deserializer::new(&mut deserializer);
    Value::deserialize(deserializer).context("parse parser AAT output")
}

/// Write `bytes` to `path` through a process-specific temporary file, creating
/// parent directories, so a reader never sees a partial file.
///
/// # Errors
///
/// Returns an error when a directory or file cannot be written or renamed.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temporary, bytes).with_context(|| format!("write {}", path.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("publish {}", path.display()))
}

/// A tool that ran and exited unsuccessfully.
#[derive(Debug)]
pub struct ToolFailed {
    pub program: String,
    pub status: String,
    pub stderr: String,
}

impl fmt::Display for ToolFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} exited with {}", self.program, self.status)
    }
}

impl std::error::Error for ToolFailed {}

/// Run `program` with `args` to completion, feeding `stdin`; the process is
/// killed once `timeout` elapses.
///
/// # Errors
///
/// Returns an error when the program cannot be spawned or times out.
pub fn run_tool<I, S>(
    program: &Path,
    args: I,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<std::process::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", program.display()))?;
    let writer = child
        .stdin
        .take()
        .zip(stdin.map(<[u8]>::to_vec))
        .map(|(mut pipe, input)| std::thread::spawn(move || pipe.write_all(&input)));
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                pipe.read_to_end(&mut bytes).map(|_| bytes)
            } else {
                Ok(bytes)
            }
        })
    };
    let stdout = read(
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    let stderr = read(
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().context("wait for tool")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{} timed out after {timeout:?}", program.display());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let join = |handle: std::thread::JoinHandle<std::io::Result<Vec<u8>>>| {
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("tool output reader panicked"))?
            .context("read tool output")
    };
    Ok(std::process::Output {
        status,
        stdout: join(stdout)?,
        stderr: join(stderr)?,
    })
}

/// [`run_tool`], returning stdout only when the program succeeds.
///
/// # Errors
///
/// Returns the errors of [`run_tool`], and a [`ToolFailed`] when the program
/// exits unsuccessfully.
pub fn run_checked<I, S>(
    program: &Path,
    args: I,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_tool(program, args, stdin, timeout)?;
    if !output.status.success() {
        return Err(ToolFailed {
            program: program.display().to_string(),
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
        .into());
    }
    Ok(output.stdout)
}
