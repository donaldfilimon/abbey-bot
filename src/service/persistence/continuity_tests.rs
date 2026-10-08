//! Canonical lineage must follow the exact owned bytes, never unrelated disk data.
use super::*;
struct DifferentBytes;
impl PersistenceSink for DifferentBytes {
    fn publish(
        &self,
        dir: &std::path::Path,
        path: &std::path::Path,
        bytes: &[u8],
    ) -> Result<(), crate::persist::PersistErrorCategory> {
        if path
            .file_name()
            .is_some_and(|f| f == crate::persist::STATE_FILE)
        {
            crate::persist::FsPersistenceSink.publish(dir, path, b"{\"external_change\":true}")
        } else {
            crate::persist::FsPersistenceSink.publish(dir, path, bytes)
        }
    }
}
#[test]
fn continuity_pending_publication_never_adopts_unrelated_disk_lineage() {
    let dir = std::env::temp_dir().join(format!("abbey-continuity-lineage-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _remove = Remove(dir.clone());
    let mut observed = false;
    write_snapshot_published(
        Some(&dir),
        &DifferentBytes,
        Snapshot {
            stores: Stores::default(),
            recall: Recall::new(),
        },
        |_| observed = true,
    );
    assert!(Stores::load(&dir).is_ok());
    assert!(
        !observed,
        "unrelated valid JSON cannot become the next snapshot's parent"
    );
}
