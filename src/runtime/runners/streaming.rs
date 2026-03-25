use std::time::{Duration, Instant};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc::{self, error::TryRecvError},
};

use crate::{
    error::{McpSubagentError, Result},
    runtime::runners::{RunnerOutputObserver, RunnerOutputStream},
};

pub struct StreamingOutput {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

async fn read_stream_chunks<R>(
    mut reader: R,
    stream: RunnerOutputStream,
    tx: mpsc::UnboundedSender<(RunnerOutputStream, Vec<u8>)>,
) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut all = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let bytes = chunk[..read].to_vec();
        all.extend_from_slice(&bytes);
        let _ = tx.send((stream, bytes));
    }
    Ok(all)
}

async fn join_chunk_task(
    task: Option<tokio::task::JoinHandle<std::io::Result<Vec<u8>>>>,
) -> Result<Vec<u8>> {
    match task {
        Some(task) => task
            .await
            .map_err(|err| McpSubagentError::Io(std::io::Error::other(err.to_string())))?
            .map_err(McpSubagentError::Io),
        None => Ok(Vec::new()),
    }
}

pub async fn collect_streaming_output(
    child: &mut tokio::process::Child,
    timeout: Duration,
    observer: &mut dyn RunnerOutputObserver,
) -> Result<StreamingOutput> {
    let (tx, mut rx) = mpsc::unbounded_channel::<(RunnerOutputStream, Vec<u8>)>();
    let stdout_task = child.stdout.take().map(|stdout| {
        let tx = tx.clone();
        tokio::spawn(read_stream_chunks(stdout, RunnerOutputStream::Stdout, tx))
    });
    let stderr_task = child.stderr.take().map(|stderr| {
        let tx = tx.clone();
        tokio::spawn(read_stream_chunks(stderr, RunnerOutputStream::Stderr, tx))
    });
    drop(tx);

    let started = Instant::now();
    let mut status: Option<std::process::ExitStatus> = None;
    let mut channel_closed = false;
    let mut timed_out = false;

    loop {
        loop {
            match rx.try_recv() {
                Ok((stream, chunk)) => {
                    if !chunk.is_empty() {
                        let text = String::from_utf8_lossy(&chunk);
                        observer.on_output(stream, text.as_ref());
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    channel_closed = true;
                    break;
                }
            }
        }

        if status.is_none() {
            status = child.try_wait().map_err(McpSubagentError::Io)?;
        }

        if status.is_some() && channel_closed {
            break;
        }

        if started.elapsed() >= timeout {
            timed_out = true;
            break;
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    if timed_out {
        let _ = child.kill().await;
        status = Some(child.wait().await.map_err(McpSubagentError::Io)?);
    }

    let stdout = join_chunk_task(stdout_task).await?;
    let stderr = join_chunk_task(stderr_task).await?;
    let status = status
        .ok_or_else(|| McpSubagentError::Io(std::io::Error::other("missing child status")))?;

    Ok(StreamingOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout).to_string(),
        stderr: String::from_utf8_lossy(&stderr).to_string(),
        timed_out,
    })
}
