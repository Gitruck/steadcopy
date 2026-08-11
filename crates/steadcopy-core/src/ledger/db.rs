//! 任务台账：SQLite 持久化。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/task-ledger/spec.md`
//! → Requirement: 任务台账持久化 / 文件级明细 / 台账查询 / 数据库位置与迁移
//!
//! 铁律：**任务完成后 MUST NOT 删除记录。** 前身「任务完成即删」与行业需求背道而驰——
//! 用户日后追问「我那天到底拷没拷」时，台账要能回答。
//!
//! 与 manifest 的分工：台账是「本机做过哪些任务」，manifest 是「这批数据是什么、怎么校验的」。
//! 两者可独立使用——删台账不影响依 manifest 复验，目的地被移走不影响台账完整。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::manifest::store::format_time;

/// 台账 schema 版本。升级 MUST 保留既有记录。
pub const SCHEMA_VERSION: i64 = 1;

const DB_NAME: &str = "ledger.db";

#[derive(Debug)]
pub enum LedgerError {
    /// 数据库打不开或损坏。**MUST NOT 静默重建**——历史无声消失比报错难受得多
    Open { path: PathBuf, reason: String },
    Query(String),
    /// schema 版本高于本程序可识别
    FutureSchema { found: i64, supported: i64 },
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedgerError::Open { path, reason } => write!(
                f,
                "任务台账打不开（{}）：{reason}。\
                 请确认文件未被占用；若确已损坏，把它改名备份后本程序会新建一份，\
                 但**旧的历史不会自动恢复**，请先留好备份",
                path.display()
            ),
            LedgerError::Query(e) => write!(f, "台账查询失败：{e}"),
            LedgerError::FutureSchema { found, supported } => write!(
                f,
                "任务台账由更新版本的程序建立（结构版本 {found}，本程序支持到 {supported}），请升级后再打开"
            ),
        }
    }
}

impl std::error::Error for LedgerError {}

/// 任务的最终状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 全部成功
    Ok,
    /// 部分失败——界面 MUST NOT 把它呈现为「完成」
    Partial,
    Cancelled,
    /// 一个都没成
    Failed,
}

impl TaskStatus {
    pub const fn label(self) -> &'static str {
        match self {
            TaskStatus::Ok => "全部通过",
            TaskStatus::Partial => "部分失败",
            TaskStatus::Cancelled => "已取消",
            TaskStatus::Failed => "失败",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Ok => "ok",
            TaskStatus::Partial => "partial",
            TaskStatus::Cancelled => "cancelled",
            TaskStatus::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "ok" => TaskStatus::Ok,
            "cancelled" => TaskStatus::Cancelled,
            "failed" => TaskStatus::Failed,
            _ => TaskStatus::Partial,
        }
    }
}

/// 一条任务记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub started_at: String,
    pub finished_at: String,
    pub source_id: String,
    pub source_name: String,
    pub project: String,
    pub algorithm: String,
    pub verified: bool,
    pub total_files: u64,
    pub total_bytes: u64,
    pub copied: u64,
    pub skipped: u64,
    pub failed: u64,
    pub status: TaskStatus,
    pub elapsed_secs: u64,
    /// 本次落下的清单路径
    pub manifests: Vec<String>,
}

/// 文件级明细。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub relative_path: String,
    pub size: u64,
    pub hash: String,
    /// copied / skipped / failed
    pub status: String,
    pub reason: Option<String>,
    pub retries: u32,
}

/// 查询条件。
#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub source_id: Option<String>,
    pub project: Option<String>,
    /// 只看有失败的
    pub only_failed: bool,
    pub since: Option<OffsetDateTime>,
    pub limit: Option<u32>,
}

/// 台账数据库位置。落**用户数据目录**，不是安装目录。
pub fn ledger_path() -> PathBuf {
    crate::config::config_dir().join(DB_NAME)
}

#[derive(Debug)]
pub struct Ledger {
    conn: Connection,
}

impl Ledger {
    pub fn open_default() -> Result<Self, LedgerError> {
        Self::open(&ledger_path())
    }

    pub fn open(path: &Path) -> Result<Self, LedgerError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LedgerError::Open {
                path: path.to_path_buf(),
                reason: e.to_string(),
            })?;
        }
        let conn = Connection::open(path).map_err(|e| LedgerError::Open {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        let me = Self { conn };
        me.migrate(path)?;
        Ok(me)
    }

    /// 内存库，测试用。
    pub fn open_in_memory() -> Result<Self, LedgerError> {
        let conn = Connection::open_in_memory().map_err(|e| LedgerError::Open {
            path: PathBuf::from(":memory:"),
            reason: e.to_string(),
        })?;
        let me = Self { conn };
        me.migrate(Path::new(":memory:"))?;
        Ok(me)
    }

    fn migrate(&self, path: &Path) -> Result<(), LedgerError> {
        let err = |e: rusqlite::Error| LedgerError::Open {
            path: path.to_path_buf(),
            reason: e.to_string(),
        };

        let found: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(err)?;
        if found > SCHEMA_VERSION {
            return Err(LedgerError::FutureSchema {
                found,
                supported: SCHEMA_VERSION,
            });
        }

        // v0 → v1：建表。后续版本在此追加分支，**只加不改**，保证既有记录不丢。
        if found < 1 {
            self.conn
                .execute_batch(
                    r#"
                    CREATE TABLE IF NOT EXISTS tasks (
                        id TEXT PRIMARY KEY,
                        started_at TEXT NOT NULL,
                        finished_at TEXT NOT NULL,
                        source_id TEXT NOT NULL,
                        source_name TEXT NOT NULL,
                        project TEXT NOT NULL,
                        algorithm TEXT NOT NULL,
                        verified INTEGER NOT NULL,
                        total_files INTEGER NOT NULL,
                        total_bytes INTEGER NOT NULL,
                        copied INTEGER NOT NULL,
                        skipped INTEGER NOT NULL,
                        failed INTEGER NOT NULL,
                        status TEXT NOT NULL,
                        elapsed_secs INTEGER NOT NULL,
                        manifests TEXT NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_tasks_time ON tasks(finished_at DESC);
                    CREATE INDEX IF NOT EXISTS idx_tasks_source ON tasks(source_id);
                    CREATE INDEX IF NOT EXISTS idx_tasks_project ON tasks(project);
                    CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);

                    CREATE TABLE IF NOT EXISTS task_files (
                        task_id TEXT NOT NULL,
                        relative_path TEXT NOT NULL,
                        size INTEGER NOT NULL,
                        hash TEXT NOT NULL,
                        status TEXT NOT NULL,
                        reason TEXT,
                        retries INTEGER NOT NULL
                    );
                    CREATE INDEX IF NOT EXISTS idx_files_task ON task_files(task_id);
                    CREATE INDEX IF NOT EXISTS idx_files_status ON task_files(task_id, status);

                    -- 格式化留痕：唯一销毁数据的操作，必须可追溯
                    CREATE TABLE IF NOT EXISTS format_attempts (
                        id TEXT PRIMARY KEY,
                        at TEXT NOT NULL,
                        device_id TEXT NOT NULL,
                        device_name TEXT NOT NULL,
                        trigger TEXT NOT NULL,
                        checks TEXT NOT NULL,
                        backup_task_id TEXT,
                        result TEXT NOT NULL,
                        reason TEXT
                    );
                    CREATE INDEX IF NOT EXISTS idx_format_time ON format_attempts(at DESC);
                    "#,
                )
                .map_err(err)?;
            self.conn
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(err)?;
        }
        Ok(())
    }

    /// 记一次任务。**完成即留，永不删除。**
    pub fn record_task(
        &self,
        task: &TaskRecord,
        files: &[FileRecord],
    ) -> Result<(), LedgerError> {
        let e = |x: rusqlite::Error| LedgerError::Query(x.to_string());
        let manifests = serde_json::to_string(&task.manifests).unwrap_or_else(|_| "[]".into());
        self.conn
            .execute(
                "INSERT OR REPLACE INTO tasks
                 (id, started_at, finished_at, source_id, source_name, project, algorithm,
                  verified, total_files, total_bytes, copied, skipped, failed, status,
                  elapsed_secs, manifests)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![
                    task.id,
                    task.started_at,
                    task.finished_at,
                    task.source_id,
                    task.source_name,
                    task.project,
                    task.algorithm,
                    task.verified as i64,
                    task.total_files as i64,
                    task.total_bytes as i64,
                    task.copied as i64,
                    task.skipped as i64,
                    task.failed as i64,
                    task.status.as_str(),
                    task.elapsed_secs as i64,
                    manifests,
                ],
            )
            .map_err(e)?;

        for f in files {
            self.conn
                .execute(
                    "INSERT INTO task_files
                     (task_id, relative_path, size, hash, status, reason, retries)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        task.id,
                        f.relative_path,
                        f.size as i64,
                        f.hash,
                        f.status,
                        f.reason,
                        f.retries as i64
                    ],
                )
                .map_err(e)?;
        }
        Ok(())
    }

    /// 按条件查历史，时间倒序。
    pub fn history(&self, q: &HistoryQuery) -> Result<Vec<TaskRecord>, LedgerError> {
        let mut sql = String::from(
            "SELECT id, started_at, finished_at, source_id, source_name, project, algorithm,
                    verified, total_files, total_bytes, copied, skipped, failed, status,
                    elapsed_secs, manifests FROM tasks WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = &q.source_id {
            sql.push_str(" AND source_id = ?");
            args.push(Box::new(s.clone()));
        }
        if let Some(p) = &q.project {
            sql.push_str(" AND project = ?");
            args.push(Box::new(p.clone()));
        }
        if q.only_failed {
            sql.push_str(" AND status IN ('partial','failed')");
        }
        if let Some(t) = q.since {
            sql.push_str(" AND finished_at >= ?");
            args.push(Box::new(format_time(t)));
        }
        sql.push_str(" ORDER BY finished_at DESC");
        if let Some(n) = q.limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }

        let e = |x: rusqlite::Error| LedgerError::Query(x.to_string());
        let mut stmt = self.conn.prepare(&sql).map_err(e)?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                let manifests: String = r.get(15)?;
                Ok(TaskRecord {
                    id: r.get(0)?,
                    started_at: r.get(1)?,
                    finished_at: r.get(2)?,
                    source_id: r.get(3)?,
                    source_name: r.get(4)?,
                    project: r.get(5)?,
                    algorithm: r.get(6)?,
                    verified: r.get::<_, i64>(7)? != 0,
                    total_files: r.get::<_, i64>(8)? as u64,
                    total_bytes: r.get::<_, i64>(9)? as u64,
                    copied: r.get::<_, i64>(10)? as u64,
                    skipped: r.get::<_, i64>(11)? as u64,
                    failed: r.get::<_, i64>(12)? as u64,
                    status: TaskStatus::parse(&r.get::<_, String>(13)?),
                    elapsed_secs: r.get::<_, i64>(14)? as u64,
                    manifests: serde_json::from_str(&manifests).unwrap_or_default(),
                })
            })
            .map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    /// 某个任务的文件级明细。`status_filter` 为空表示全部。
    pub fn task_files(
        &self,
        task_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<FileRecord>, LedgerError> {
        let e = |x: rusqlite::Error| LedgerError::Query(x.to_string());
        let filter = status_filter.map(str::to_string);
        let sql = if filter.is_some() {
            "SELECT relative_path, size, hash, status, reason, retries
             FROM task_files WHERE task_id = ?1 AND status = ?2 ORDER BY relative_path"
        } else {
            "SELECT relative_path, size, hash, status, reason, retries
             FROM task_files WHERE task_id = ?1 ORDER BY relative_path"
        };
        let owned_id = task_id.to_string();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(owned_id)];
        if let Some(f) = filter {
            args.push(Box::new(f));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self.conn.prepare(sql).map_err(e)?;
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                Ok(FileRecord {
                    relative_path: r.get(0)?,
                    size: r.get::<_, i64>(1)? as u64,
                    hash: r.get(2)?,
                    status: r.get(3)?,
                    reason: r.get(4)?,
                    retries: r.get::<_, i64>(5)? as u32,
                })
            })
            .map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    pub fn task(&self, id: &str) -> Result<Option<TaskRecord>, LedgerError> {
        Ok(self
            .history(&HistoryQuery::default())?
            .into_iter()
            .find(|t| t.id == id))
    }

    /// 记一次格式化尝试。**无论成功、失败、被拒还是被取消都要记。**
    #[allow(clippy::too_many_arguments)]
    pub fn record_format_attempt(
        &self,
        id: &str,
        at: OffsetDateTime,
        device_id: &str,
        device_name: &str,
        trigger: &str,
        checks: &str,
        backup_task_id: Option<&str>,
        result: &str,
        reason: Option<&str>,
    ) -> Result<(), LedgerError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO format_attempts
                 (id, at, device_id, device_name, trigger, checks, backup_task_id, result, reason)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    id,
                    format_time(at),
                    device_id,
                    device_name,
                    trigger,
                    checks,
                    backup_task_id,
                    result,
                    reason
                ],
            )
            .map_err(|x| LedgerError::Query(x.to_string()))?;
        Ok(())
    }

    /// 格式化尝试的历史（时间倒序）。
    pub fn format_attempts(&self) -> Result<Vec<FormatAttempt>, LedgerError> {
        let e = |x: rusqlite::Error| LedgerError::Query(x.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, at, device_id, device_name, trigger, checks, backup_task_id, result, reason
                 FROM format_attempts ORDER BY at DESC",
            )
            .map_err(e)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FormatAttempt {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    device_id: r.get(2)?,
                    device_name: r.get(3)?,
                    trigger: r.get(4)?,
                    checks: r.get(5)?,
                    backup_task_id: r.get(6)?,
                    result: r.get(7)?,
                    reason: r.get(8)?,
                })
            })
            .map_err(e)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
    }

    /// 清空台账。**只清本机历史**——目的地上的素材与凭证不受影响。
    pub fn clear(&self) -> Result<(), LedgerError> {
        self.conn
            .execute_batch("DELETE FROM task_files; DELETE FROM tasks; DELETE FROM format_attempts;")
            .map_err(|x| LedgerError::Query(x.to_string()))
    }

    pub fn count(&self) -> Result<u64, LedgerError> {
        self.conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
            .optional()
            .map_err(|x| LedgerError::Query(x.to_string()))
            .map(|v| v.unwrap_or(0) as u64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatAttempt {
    pub id: String,
    pub at: String,
    pub device_id: String,
    pub device_name: String,
    pub trigger: String,
    pub checks: String,
    pub backup_task_id: Option<String>,
    pub result: String,
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn task(id: &str, status: TaskStatus, project: &str) -> TaskRecord {
        TaskRecord {
            id: id.into(),
            started_at: "2026-08-10T09:00:00Z".into(),
            finished_at: format!("2026-08-10T09:{:02}:00Z", id.len()),
            source_id: "vol:1".into(),
            source_name: "A7M4主卡".into(),
            project: project.into(),
            algorithm: "xxh64".into(),
            verified: true,
            total_files: 10,
            total_bytes: 1024,
            copied: 8,
            skipped: 1,
            failed: if status == TaskStatus::Ok { 0 } else { 1 },
            status,
            elapsed_secs: 42,
            manifests: vec![r"D:\素材\steadcopy\m.json".into()],
        }
    }

    fn files() -> Vec<FileRecord> {
        vec![
            FileRecord {
                relative_path: "A001.MP4".into(),
                size: 900,
                hash: "abc".into(),
                status: "copied".into(),
                reason: None,
                retries: 0,
            },
            FileRecord {
                relative_path: "A002.MP4".into(),
                size: 124,
                hash: "def".into(),
                status: "failed".into(),
                reason: Some("校验不一致：期望 def，实际 xyz".into()),
                retries: 2,
            },
        ]
    }

    // spec: task-ledger → 任务台账持久化 → Scenario: 完成的任务留在台账
    #[test]
    fn scenario_task_ledger_record_survives_reopen() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("ledger.db");
        {
            let l = Ledger::open(&p).expect("建库");
            l.record_task(&task("t1", TaskStatus::Ok, "婚礼"), &files())
                .expect("记录");
        }
        // 重开：记录必须还在
        let l = Ledger::open(&p).expect("重开");
        let all = l.history(&HistoryQuery::default()).expect("查询");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "t1");
        assert_eq!(all[0].source_name, "A7M4主卡");
        assert_eq!(all[0].manifests.len(), 1);
    }

    // spec: → Scenario: 失败与取消的任务同样留档
    #[test]
    fn scenario_task_ledger_failed_and_cancelled_are_kept() {
        let l = Ledger::open_in_memory().expect("建库");
        for (id, st) in [
            ("t-ok", TaskStatus::Ok),
            ("t-partial", TaskStatus::Partial),
            ("t-cancel", TaskStatus::Cancelled),
            ("t-fail", TaskStatus::Failed),
        ] {
            l.record_task(&task(id, st, "婚礼"), &[]).expect("记录");
        }
        assert_eq!(l.count().expect("计数"), 4, "四种最终状态都要留档");
    }

    // spec: → 文件级明细 → Scenario: 失败文件可定位
    #[test]
    fn scenario_task_ledger_failed_files_are_locatable() {
        let l = Ledger::open_in_memory().expect("建库");
        l.record_task(&task("t1", TaskStatus::Partial, "婚礼"), &files())
            .expect("记录");

        let failed = l.task_files("t1", Some("failed")).expect("查明细");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].relative_path, "A002.MP4");
        assert_eq!(failed[0].retries, 2);
        assert!(failed[0]
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("校验不一致")));

        assert_eq!(l.task_files("t1", None).expect("全部").len(), 2);
    }

    // spec: → 台账查询 → Scenario: 按设备筛选 / 按状态筛选
    #[test]
    fn scenario_task_ledger_query_filters() {
        let l = Ledger::open_in_memory().expect("建库");
        l.record_task(&task("t1", TaskStatus::Ok, "婚礼"), &[]).expect("记");
        l.record_task(&task("t22", TaskStatus::Partial, "广告"), &[]).expect("记");

        let by_project = l
            .history(&HistoryQuery {
                project: Some("广告".into()),
                ..Default::default()
            })
            .expect("查");
        assert_eq!(by_project.len(), 1);
        assert_eq!(by_project[0].id, "t22");

        let failed = l
            .history(&HistoryQuery {
                only_failed: true,
                ..Default::default()
            })
            .expect("查");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].status, TaskStatus::Partial);

        let by_device = l
            .history(&HistoryQuery {
                source_id: Some("vol:1".into()),
                ..Default::default()
            })
            .expect("查");
        assert_eq!(by_device.len(), 2);
    }

    #[test]
    fn scenario_task_ledger_history_is_time_desc() {
        let l = Ledger::open_in_memory().expect("建库");
        // finished_at 由 id 长度派生，t1 < t22 < t333
        l.record_task(&task("t1", TaskStatus::Ok, "a"), &[]).expect("记");
        l.record_task(&task("t333", TaskStatus::Ok, "b"), &[]).expect("记");
        l.record_task(&task("t22", TaskStatus::Ok, "c"), &[]).expect("记");
        let all = l.history(&HistoryQuery::default()).expect("查");
        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["t333", "t22", "t1"], "应按时间倒序");
    }

    #[test]
    fn scenario_task_ledger_limit() {
        let l = Ledger::open_in_memory().expect("建库");
        for i in 0..5 {
            l.record_task(&task(&format!("{}", "t".repeat(i + 1)), TaskStatus::Ok, "x"), &[])
                .expect("记");
        }
        let some = l
            .history(&HistoryQuery {
                limit: Some(2),
                ..Default::default()
            })
            .expect("查");
        assert_eq!(some.len(), 2);
    }

    // spec: → 数据库位置与迁移 → Scenario: 升级保留历史
    #[test]
    fn scenario_task_ledger_migration_keeps_records() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("ledger.db");
        {
            let l = Ledger::open(&p).expect("建库");
            l.record_task(&task("t1", TaskStatus::Ok, "婚礼"), &files())
                .expect("记");
        }
        // 再开一次会重跑 migrate，记录必须原样在
        let l = Ledger::open(&p).expect("重开");
        assert_eq!(l.count().expect("计数"), 1);
        assert_eq!(l.task_files("t1", None).expect("明细").len(), 2);
    }

    // spec: → Scenario: 数据库损坏不静默重建
    #[test]
    fn scenario_task_ledger_future_schema_rejected() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("ledger.db");
        {
            let c = Connection::open(&p).expect("建库");
            c.pragma_update(None, "user_version", SCHEMA_VERSION + 5)
                .expect("设版本");
        }
        let err = Ledger::open(&p).expect_err("未来版本 MUST 被拒");
        assert!(matches!(err, LedgerError::FutureSchema { .. }));
        assert!(err.to_string().contains("升级"));
        assert!(p.exists(), "MUST NOT 删掉用户的库");
    }

    #[test]
    fn scenario_task_ledger_corrupt_db_reports_readably() {
        let dir = tempfile::tempdir().expect("临时目录");
        let p = dir.path().join("ledger.db");
        std::fs::write(&p, b"this is definitely not a sqlite file").expect("写垃圾");
        let err = Ledger::open(&p).expect_err("损坏库 MUST 报错");
        let msg = err.to_string();
        assert!(msg.contains("台账"), "错误信息应说人话：{msg}");
        assert!(msg.contains("备份"), "应提示先留备份：{msg}");
        assert!(p.exists(), "MUST NOT 静默删除或重建用户的库");
    }

    // spec: → 台账与 manifest 的职责分离 → Scenario: 台账被清空后复验仍可用
    #[test]
    fn scenario_task_ledger_clear_only_touches_local_history() {
        let l = Ledger::open_in_memory().expect("建库");
        l.record_task(&task("t1", TaskStatus::Ok, "婚礼"), &files())
            .expect("记");
        assert_eq!(l.count().expect("计数"), 1);
        l.clear().expect("清空");
        assert_eq!(l.count().expect("计数"), 0);
        assert!(l.task_files("t1", None).expect("明细").is_empty());
        // manifest 在目的地上，与本库无关——这里断言的是「清空只影响本库」
    }

    #[test]
    fn scenario_task_ledger_format_attempts_are_recorded() {
        let l = Ledger::open_in_memory().expect("建库");
        let at = datetime!(2026-08-10 09:30:00 UTC);
        l.record_format_attempt(
            "f1", at, "vol:1", "A7M4主卡", "manual", "G1=ok;G4=fail",
            Some("t1"), "rejected", Some("备份记录未覆盖当前全部内容"),
        )
        .expect("记");
        l.record_format_attempt("f2", at, "vol:1", "A7M4主卡", "auto", "all=ok", Some("t1"), "ok", None)
            .expect("记");

        let all = l.format_attempts().expect("查");
        assert_eq!(all.len(), 2, "被拒的尝试同样要留痕");
        assert!(all.iter().any(|a| a.result == "rejected"));
        assert!(all
            .iter()
            .any(|a| a.reason.as_deref() == Some("备份记录未覆盖当前全部内容")));
    }

    #[test]
    fn scenario_task_ledger_path_is_in_user_data_dir() {
        let p = ledger_path();
        assert!(p.ends_with(DB_NAME));
        assert_eq!(p.parent(), Some(crate::config::config_dir().as_path()));
    }
}
