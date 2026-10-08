//! Synthetic process proof: request cancellation kills and waits without shutdown.
use super::*;
use crate::service::{OperationKind, OwnedTaskKind, ServiceSupervisor};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[tokio::test]
async fn attempt_cancellation_joins_actual_process_and_releases_private_file() {
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let registry = supervisor.operations();
    let fm = FoundationModels::new(
        FmConfig {
            mode: FmMode::System,
            endpoint: None,
            cli: "/bin/sh".into(),
            fallback: false,
            primary: false,
            timeout_secs: 60,
        },
        None,
        false,
    );
    fm.attach_service(registry.clone());
    let marker = std::env::temp_dir().join(format!("abbey-fm-cancel-pid-{}", std::process::id()));
    let file = PrivateSchemaFile::create(&json!({})).unwrap();
    let schema = file.path().to_owned();
    let dropped = Arc::new(AtomicBool::new(false));
    struct Owner(PrivateSchemaFile, Arc<AtomicBool>);
    impl Drop for Owner {
        fn drop(&mut self) {
            assert!(self.0.path().exists());
            self.1.store(true, Ordering::SeqCst);
        }
    }
    let invocation = CliInvocation {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "echo $$ > \"$1\"; exec /bin/sleep 60".into(),
            "synthetic".into(),
            marker.as_os_str().to_owned(),
        ],
        stdin: Vec::new(),
        environment: Vec::new(),
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let work = Box::pin(fm.run_owned(
        invocation,
        Owner(file, dropped.clone()),
        Some(cancel.clone()),
    ));
    let witness = async {
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&marker)
                    && let Ok(pid) = text.trim().parse::<u32>()
                {
                    break pid;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(schema.exists());
        assert!(!dropped.load(Ordering::SeqCst));
        cancel.cancel();
        pid
    };
    let (result, pid) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(work, witness)
    })
    .await
    .unwrap();
    assert_eq!(
        result.unwrap_err().provider_failure(),
        ProviderFailureKind::Cancelled
    );
    let joined = supervisor.next_completion().await;
    assert_eq!(
        joined.kind,
        OwnedTaskKind::Operation(OperationKind::ProviderProcess)
    );
    assert!(supervisor.outstanding().is_empty());
    assert!(!registry.cancellation().is_cancelled());
    assert!(dropped.load(Ordering::SeqCst));
    assert!(!schema.exists());
    assert!(
        !std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    std::fs::remove_file(marker).unwrap();
}
