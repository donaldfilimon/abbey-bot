//! Retained ABI child execution and bounded outcome decoding.
use super::*;

pub(super) async fn run_abi(program: &Path, args: &[OsString], timeout_secs: u64) -> GateOutcome {
    run_abi_owned(program, args, timeout_secs, None).await
}
pub(super) async fn run_abi_owned(
    program: &Path,
    args: &[OsString],
    timeout_secs: u64,
    cancel: Option<tokio_util::sync::CancellationToken>,
) -> GateOutcome {
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| {
            ALLOWED_ENVIRONMENT
                .iter()
                .any(|allowed| name == OsStr::new(allowed))
        })
        .collect();
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(environment.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return GateOutcome::Unavailable {
                detail: format!("could not start the abi binary: {error}"),
            };
        }
    };
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.start_kill();
        let _ = child.wait().await;
        return GateOutcome::Unavailable {
            detail: "the abi child had no output pipes".into(),
        };
    };
    let operation = async {
        let (stdout, stderr, status) = tokio::try_join!(
            read_capped(stdout, MAX_STDOUT_BYTES),
            read_capped(stderr, MAX_STDERR_BYTES),
            async { child.wait().await.map_err(|error| error.to_string()) },
        )?;
        Ok::<_, String>((stdout, stderr, status))
    };
    let completed = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(timeout_secs), operation) => Some(result),
        () = async { match cancel { Some(cancel) => cancel.cancelled().await, None => std::future::pending().await } } => None,
    };
    let (stdout, stderr, status) = match completed {
        Some(Ok(Ok(result))) => result,
        failure => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return GateOutcome::Unavailable {
                detail: match failure {
                    Some(Ok(Err(detail))) => detail,
                    Some(Err(_)) => format!("the abi binary did not answer within {timeout_secs}s"),
                    None => "the abi operation was cancelled during shutdown".into(),
                    Some(Ok(Ok(_))) => unreachable!("success handled above"),
                },
            };
        }
    };
    classify(status.code(), &stdout, &stderr)
}

async fn read_capped(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > limit {
            return Err(format!("the abi binary's output exceeded {limit} bytes"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}
