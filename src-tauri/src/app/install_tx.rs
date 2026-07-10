//! Crash-safe install transaction log + startup recovery.
//!
//! Before the first destructive rename (old install → backup), a durable log is
//! written. On the next launch we scan pending logs and decide:
//!   - **continue** — finish moving the staged payload into place
//!   - **rollback** — restore the backup
//!   - **keep** — leave materials for manual recovery when the matrix is ambiguous
//!
//! Recovery always runs before ordinary staging/backup cleanup.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::app::atomic_file;
use crate::app::paths;
use crate::errors::AppError;

pub const SCHEMA_VERSION: u32 = 1;

/// Platform / path kind of a destructive install swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallTxKind {
    MacosSwap,
    WindowsPortable,
}

impl InstallTxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MacosSwap => "macos-swap",
            Self::WindowsPortable => "windows-portable",
        }
    }
}

/// Durable step markers written across rename boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallTxStep {
    /// Log persisted; no destructive rename yet.
    Prepared,
    /// Old install moved to backup; install path is empty (or missing).
    OldMoved,
    /// New payload moved into install path.
    NewInstalled,
    /// Success path finished; log may be deleted.
    Completed,
    /// Backup restored over install path.
    RolledBack,
    /// Ambiguous on-disk state; materials retained for manual recovery.
    NeedsManual,
}

impl InstallTxStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::OldMoved => "old-moved",
            Self::NewInstalled => "new-installed",
            Self::Completed => "completed",
            Self::RolledBack => "rolled-back",
            Self::NeedsManual => "needs-manual",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::RolledBack | Self::NeedsManual
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallTransaction {
    #[serde(default = "default_schema")]
    pub schema_version: u32,
    pub id: String,
    pub kind: InstallTxKind,
    pub step: InstallTxStep,
    pub install_path: String,
    pub new_path: String,
    pub backup_path: String,
    pub had_previous: bool,
    #[serde(default)]
    pub was_running: Option<bool>,
    pub started_unix: u64,
    pub updated_unix: u64,
    #[serde(default)]
    pub notes: Vec<String>,
}

fn default_schema() -> u32 {
    SCHEMA_VERSION
}

/// Pure recovery decision for the macOS swap / Windows portable rename matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// No damage yet — drop the log.
    ClearLog,
    /// Rename staged new → install, then clean up.
    ContinueInstall,
    /// Rename backup → install.
    Rollback,
    /// Install already good; drop backup if present and clear log.
    Complete,
    /// Leave paths + log (marked needs-manual) for human inspection.
    KeepManual { reason: &'static str },
}

/// Decide recovery from the durable step + which paths still exist.
///
/// Matrix (macOS swap and Windows portable share the same two-rename shape):
///
/// | step          | install | backup | new | action            |
/// |---------------|---------|--------|-----|-------------------|
/// | prepared      | *       | *      | *   | clear log         |
/// | old-moved     | no      | yes    | yes | continue          |
/// | old-moved     | no      | yes    | no  | rollback          |
/// | old-moved     | no      | no     | yes | keep (no backup)  |
/// | old-moved     | no      | no     | no  | keep (all missing)|
/// | old-moved     | yes     | *      | *   | complete if prior |
/// | new-installed | yes     | *      | *   | complete          |
/// | new-installed | no      | yes    | *   | rollback          |
/// | new-installed | no      | no     | *   | keep              |
/// | terminal      | *       | *      | *   | clear / keep note |
pub fn decide_recovery(
    step: InstallTxStep,
    install_exists: bool,
    backup_exists: bool,
    new_exists: bool,
) -> RecoveryAction {
    match step {
        InstallTxStep::Prepared => RecoveryAction::ClearLog,
        InstallTxStep::Completed | InstallTxStep::RolledBack => RecoveryAction::ClearLog,
        InstallTxStep::NeedsManual => RecoveryAction::KeepManual {
            reason: "previous recovery already marked needs-manual",
        },
        InstallTxStep::OldMoved => match (install_exists, backup_exists, new_exists) {
            (false, true, true) => RecoveryAction::ContinueInstall,
            (false, true, false) => RecoveryAction::Rollback,
            (false, false, true) => RecoveryAction::KeepManual {
                reason: "old moved aside but backup missing; staged new retained",
            },
            (false, false, false) => RecoveryAction::KeepManual {
                reason: "install, backup, and staged new all missing after old-moved",
            },
            (true, _, _) => RecoveryAction::Complete,
        },
        InstallTxStep::NewInstalled => match (install_exists, backup_exists) {
            (true, _) => RecoveryAction::Complete,
            (false, true) => RecoveryAction::Rollback,
            (false, false) => RecoveryAction::KeepManual {
                reason: "new was installed but install path missing and no backup",
            },
        },
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn transactions_dir() -> Option<PathBuf> {
    paths::data_dir().map(|dir| dir.join("install-transactions"))
}

pub fn tx_path_for(id: &str) -> Option<PathBuf> {
    transactions_dir().map(|dir| dir.join(format!("{id}.json")))
}

impl InstallTransaction {
    pub fn begin(
        kind: InstallTxKind,
        install_path: &Path,
        new_path: &Path,
        backup_path: &Path,
        had_previous: bool,
        was_running: Option<bool>,
    ) -> Result<Self, AppError> {
        let now = now_unix();
        let tx = Self {
            schema_version: SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            step: InstallTxStep::Prepared,
            install_path: install_path.to_string_lossy().into_owned(),
            new_path: new_path.to_string_lossy().into_owned(),
            backup_path: backup_path.to_string_lossy().into_owned(),
            had_previous,
            was_running,
            started_unix: now,
            updated_unix: now,
            notes: Vec::new(),
        };
        tx.persist()?;
        log::info!(
            "install transaction prepared id={} kind={} install={}",
            tx.id,
            kind.as_str(),
            tx.install_path
        );
        Ok(tx)
    }

    pub fn persist(&self) -> Result<(), AppError> {
        let path = tx_path_for(&self.id).ok_or_else(|| {
            AppError::Internal("无法定位 install-transactions 数据目录".to_string())
        })?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::Internal(format!("创建事务日志目录失败: {e}")))?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| AppError::Internal(format!("序列化事务日志失败: {e}")))?;
        atomic_file::write_atomic(&path, &bytes)
            .map_err(|e| AppError::Internal(format!("写入事务日志失败: {e}")))?;
        Ok(())
    }

    pub fn advance(&mut self, step: InstallTxStep) -> Result<(), AppError> {
        self.step = step;
        self.updated_unix = now_unix();
        self.persist()?;
        log::info!(
            "install transaction step id={} step={}",
            self.id,
            step.as_str()
        );
        Ok(())
    }

    pub fn note(&mut self, message: impl Into<String>) -> Result<(), AppError> {
        self.notes.push(message.into());
        self.updated_unix = now_unix();
        self.persist()
    }

    pub fn complete(mut self) -> Result<(), AppError> {
        self.step = InstallTxStep::Completed;
        self.updated_unix = now_unix();
        // Persist terminal state briefly so a crash mid-delete still recovers cleanly,
        // then remove the file.
        let _ = self.persist();
        self.remove_file();
        Ok(())
    }

    pub fn remove_file(&self) {
        if let Some(path) = tx_path_for(&self.id) {
            let _ = fs::remove_file(path);
        }
    }

    pub fn load_from_path(path: &Path) -> Result<Self, AppError> {
        let bytes = fs::read(path)
            .map_err(|e| AppError::Internal(format!("读取事务日志失败: {e}")))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| AppError::Internal(format!("解析事务日志失败: {e}")))
    }
}

#[derive(Debug, Clone, Default)]
pub struct RecoverySummary {
    pub scanned: usize,
    pub continued: usize,
    pub rolled_back: usize,
    pub completed: usize,
    pub cleared: usize,
    pub kept_manual: usize,
    pub failed: usize,
}

/// Scan pending transaction logs and apply the recovery matrix. Must run
/// **before** ordinary staging cleanup so recovery materials are not deleted.
pub fn recover_pending_transactions() -> RecoverySummary {
    let mut summary = RecoverySummary::default();
    let Some(dir) = transactions_dir() else {
        return summary;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return summary;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        summary.scanned += 1;
        match recover_one(&path) {
            Ok(outcome) => match outcome {
                Recovered::Continued => summary.continued += 1,
                Recovered::RolledBack => summary.rolled_back += 1,
                Recovered::Completed => summary.completed += 1,
                Recovered::Cleared => summary.cleared += 1,
                Recovered::KeptManual => summary.kept_manual += 1,
            },
            Err(err) => {
                summary.failed += 1;
                log::error!(
                    "install transaction recovery failed path={} error={err}",
                    path.display()
                );
            }
        }
    }
    if summary.scanned > 0 {
        log::info!(
            "install transaction recovery summary scanned={} continued={} rolled_back={} completed={} cleared={} kept_manual={} failed={}",
            summary.scanned,
            summary.continued,
            summary.rolled_back,
            summary.completed,
            summary.cleared,
            summary.kept_manual,
            summary.failed
        );
    }
    summary
}

#[derive(Debug)]
enum Recovered {
    Continued,
    RolledBack,
    Completed,
    Cleared,
    KeptManual,
}

fn path_exists(p: &str) -> bool {
    Path::new(p).exists()
}

fn recover_one(path: &Path) -> Result<Recovered, AppError> {
    let mut tx = InstallTransaction::load_from_path(path)?;
    if tx.step.is_terminal() && matches!(tx.step, InstallTxStep::Completed | InstallTxStep::RolledBack)
    {
        let _ = fs::remove_file(path);
        return Ok(Recovered::Cleared);
    }

    let install_exists = path_exists(&tx.install_path);
    let backup_exists = path_exists(&tx.backup_path);
    let new_exists = path_exists(&tx.new_path);
    let action = decide_recovery(tx.step, install_exists, backup_exists, new_exists);
    log::info!(
        "install transaction recover id={} step={} action={:?} install={} backup={} new={}",
        tx.id,
        tx.step.as_str(),
        action,
        install_exists,
        backup_exists,
        new_exists
    );

    match action {
        RecoveryAction::ClearLog => {
            let _ = fs::remove_file(path);
            Ok(Recovered::Cleared)
        }
        RecoveryAction::ContinueInstall => {
            fs::rename(&tx.new_path, &tx.install_path).map_err(|e| {
                AppError::Internal(format!("recovery continue rename failed: {e}"))
            })?;
            tx.advance(InstallTxStep::NewInstalled)?;
            cleanup_backup_best_effort(&tx);
            tx.complete()?;
            Ok(Recovered::Continued)
        }
        RecoveryAction::Rollback => {
            if path_exists(&tx.install_path) {
                let _ = fs::remove_dir_all(&tx.install_path);
                let _ = fs::remove_file(&tx.install_path);
            }
            fs::rename(&tx.backup_path, &tx.install_path).map_err(|e| {
                AppError::Internal(format!("recovery rollback rename failed: {e}"))
            })?;
            tx.step = InstallTxStep::RolledBack;
            tx.updated_unix = now_unix();
            let _ = tx.persist();
            tx.remove_file();
            Ok(Recovered::RolledBack)
        }
        RecoveryAction::Complete => {
            cleanup_backup_best_effort(&tx);
            // Staged new should already be gone; remove if leftover.
            if path_exists(&tx.new_path) {
                let _ = fs::remove_dir_all(&tx.new_path);
                let _ = fs::remove_file(&tx.new_path);
            }
            tx.complete()?;
            Ok(Recovered::Completed)
        }
        RecoveryAction::KeepManual { reason } => {
            tx.step = InstallTxStep::NeedsManual;
            tx.updated_unix = now_unix();
            tx.notes.push(reason.to_string());
            tx.persist()?;
            log::error!(
                "install transaction needs manual recovery id={} reason={reason} install={} backup={} new={}",
                tx.id,
                tx.install_path,
                tx.backup_path,
                tx.new_path
            );
            Ok(Recovered::KeptManual)
        }
    }
}

fn cleanup_backup_best_effort(tx: &InstallTransaction) {
    if path_exists(&tx.backup_path) {
        let _ = fs::remove_dir_all(&tx.backup_path);
        let _ = fs::remove_file(&tx.backup_path);
    }
}

/// RAII helper used by perform paths: advances steps and completes/clears on drop
/// only if still non-terminal (failure path leaves the log for startup recovery).
pub struct ActiveInstallTx {
    inner: Option<InstallTransaction>,
}

impl ActiveInstallTx {
    pub fn begin(
        kind: InstallTxKind,
        install_path: &Path,
        new_path: &Path,
        backup_path: &Path,
        had_previous: bool,
        was_running: Option<bool>,
    ) -> Result<Self, AppError> {
        let tx = InstallTransaction::begin(
            kind,
            install_path,
            new_path,
            backup_path,
            had_previous,
            was_running,
        )?;
        Ok(Self { inner: Some(tx) })
    }

    pub fn mark_old_moved(&mut self) -> Result<(), AppError> {
        if let Some(tx) = self.inner.as_mut() {
            tx.advance(InstallTxStep::OldMoved)?;
        }
        Ok(())
    }

    pub fn mark_new_installed(&mut self) -> Result<(), AppError> {
        if let Some(tx) = self.inner.as_mut() {
            tx.advance(InstallTxStep::NewInstalled)?;
        }
        Ok(())
    }

    pub fn succeed(mut self) -> Result<(), AppError> {
        if let Some(tx) = self.inner.take() {
            tx.complete()?;
        }
        Ok(())
    }

    /// Explicit abort before any destructive rename — safe to delete the log.
    pub fn abort_clean(mut self) {
        if let Some(tx) = self.inner.take() {
            if matches!(tx.step, InstallTxStep::Prepared) {
                tx.remove_file();
            }
            // If we already moved files, leave the log for startup recovery.
        }
    }
}

impl Drop for ActiveInstallTx {
    fn drop(&mut self) {
        // Non-terminal logs intentionally survive process death / panic so the
        // next launch can recover. Prepared-only logs without renames are
        // cleared to avoid false-positive recovery.
        if let Some(tx) = self.inner.take() {
            if matches!(tx.step, InstallTxStep::Prepared) {
                tx.remove_file();
            } else {
                log::warn!(
                    "install transaction left pending for recovery id={} step={}",
                    tx.id,
                    tx.step.as_str()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn recovery_matrix_old_moved_boundaries() {
        assert_eq!(
            decide_recovery(InstallTxStep::OldMoved, false, true, true),
            RecoveryAction::ContinueInstall
        );
        assert_eq!(
            decide_recovery(InstallTxStep::OldMoved, false, true, false),
            RecoveryAction::Rollback
        );
        assert!(matches!(
            decide_recovery(InstallTxStep::OldMoved, false, false, true),
            RecoveryAction::KeepManual { .. }
        ));
        assert!(matches!(
            decide_recovery(InstallTxStep::OldMoved, false, false, false),
            RecoveryAction::KeepManual { .. }
        ));
        assert_eq!(
            decide_recovery(InstallTxStep::OldMoved, true, true, false),
            RecoveryAction::Complete
        );
    }

    #[test]
    fn recovery_matrix_new_installed_and_prepared() {
        assert_eq!(
            decide_recovery(InstallTxStep::NewInstalled, true, true, false),
            RecoveryAction::Complete
        );
        assert_eq!(
            decide_recovery(InstallTxStep::NewInstalled, false, true, false),
            RecoveryAction::Rollback
        );
        assert!(matches!(
            decide_recovery(InstallTxStep::NewInstalled, false, false, false),
            RecoveryAction::KeepManual { .. }
        ));
        assert_eq!(
            decide_recovery(InstallTxStep::Prepared, true, false, true),
            RecoveryAction::ClearLog
        );
        assert_eq!(
            decide_recovery(InstallTxStep::Completed, true, false, false),
            RecoveryAction::ClearLog
        );
    }

    fn test_root(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-data")
            .join(format!("install-tx-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn execute_continue_after_old_moved_crash() {
        let root = test_root("continue");
        let install = root.join("Codex.app");
        let backup = root.join("backup-Codex.app");
        let new_app = root.join("new-Codex.app");
        // Simulate crash after old→backup: install missing, backup+new present.
        fs::create_dir_all(backup.join("Contents")).unwrap();
        fs::write(backup.join("Contents/ver"), "old").unwrap();
        fs::create_dir_all(new_app.join("Contents")).unwrap();
        fs::write(new_app.join("Contents/ver"), "new").unwrap();

        let action = decide_recovery(
            InstallTxStep::OldMoved,
            install.exists(),
            backup.exists(),
            new_app.exists(),
        );
        assert_eq!(action, RecoveryAction::ContinueInstall);
        fs::rename(&new_app, &install).unwrap();
        assert_eq!(
            fs::read_to_string(install.join("Contents/ver")).unwrap(),
            "new"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn execute_rollback_when_new_missing_after_old_moved() {
        let root = test_root("rollback");
        let install = root.join("Codex");
        let backup = root.join("Codex.rollback");
        let new_app = root.join("payload");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("marker"), "old").unwrap();
        // new missing → rollback
        assert_eq!(
            decide_recovery(
                InstallTxStep::OldMoved,
                install.exists(),
                backup.exists(),
                new_app.exists()
            ),
            RecoveryAction::Rollback
        );
        fs::rename(&backup, &install).unwrap();
        assert_eq!(fs::read_to_string(install.join("marker")).unwrap(), "old");
        let _ = fs::remove_dir_all(root);
    }
}
