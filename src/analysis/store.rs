//! Durable storage for failure investigation attempts.
//!
//! Attempts live in `failure_investigations` beside the agent session data
//! (spec §8) so the asynchronous service can survive restarts, cache successful
//! verdicts by evidence fingerprint, and guarantee one in-flight job per cache
//! key. Only structured data is persisted: the filter, the evidence
//! fingerprint, scalar verdict metadata, the validated [`AnalysisResult`] and
//! [`ToolCallAudit`] rows. Raw logs, spans and tool result payloads never reach
//! the database.

use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::analysis::{AnalysisResult, Confidence, ToolCallAudit, VerdictCategory};
use crate::domain::FailureFilter;

const SCHEMA: &str = include_str!("../../migrations/001_failure_investigations.sql");

const SELECT_BY_ID: &str = "SELECT id, trace_id, filter_json, evidence_fingerprint, model, \
     prompt_version, status, verdict_category, confidence, result_json, tool_calls_json, \
     error, attempt, created_at, finished_at FROM failure_investigations WHERE id = ?";

const SELECT_LATEST: &str = "SELECT id, trace_id, filter_json, evidence_fingerprint, model, \
     prompt_version, status, verdict_category, confidence, result_json, tool_calls_json, \
     error, attempt, created_at, finished_at FROM failure_investigations WHERE trace_id = ? \
     ORDER BY created_at DESC, rowid DESC LIMIT 1";

const SELECT_CACHE_HIT: &str = "SELECT id, trace_id, filter_json, evidence_fingerprint, model, \
     prompt_version, status, verdict_category, confidence, result_json, tool_calls_json, \
     error, attempt, created_at, finished_at FROM failure_investigations \
     WHERE trace_id = ? AND evidence_fingerprint = ? AND model = ? AND prompt_version = ? \
     AND status = 'succeeded' ORDER BY attempt DESC, rowid DESC LIMIT 1";

const SELECT_RUNNING: &str = "SELECT id, trace_id, filter_json, evidence_fingerprint, model, \
     prompt_version, status, verdict_category, confidence, result_json, tool_calls_json, \
     error, attempt, created_at, finished_at FROM failure_investigations \
     WHERE trace_id = ? AND evidence_fingerprint = ? AND model = ? AND prompt_version = ? \
     AND status IN ('queued', 'running') ORDER BY created_at DESC, rowid DESC LIMIT 1";

const INSERT_SQL: &str = "INSERT INTO failure_investigations (id, trace_id, filter_json, \
     evidence_fingerprint, model, prompt_version, status, verdict_category, confidence, \
     result_json, tool_calls_json, error, attempt, created_at, finished_at) \
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

/// Task lifecycle of one investigation attempt. The result-level status lives
/// in [`crate::analysis::AnalysisStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Invalid,
}

impl InvestigationStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Invalid => "invalid",
        }
    }

    fn from_db(text: &str) -> Result<Self, String> {
        match text {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "invalid" => Ok(Self::Invalid),
            other => Err(format!("invalid investigation status {other:?}")),
        }
    }
}

/// One stored investigation attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationRecord {
    pub id: String,
    pub trace_id: String,
    pub filter: FailureFilter,
    pub evidence_fingerprint: String,
    pub model: String,
    pub prompt_version: String,
    pub status: InvestigationStatus,
    pub verdict_category: Option<VerdictCategory>,
    pub confidence: Option<Confidence>,
    pub result: Option<AnalysisResult>,
    pub tool_calls: Vec<ToolCallAudit>,
    pub error: Option<String>,
    pub attempt: u32,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

/// Lightweight row for the failure list; never loads `result_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationMarker {
    pub id: String,
    pub trace_id: String,
    pub status: InvestigationStatus,
    pub verdict_category: Option<VerdictCategory>,
    pub confidence: Option<Confidence>,
    pub attempt: u32,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

/// Everything a terminal attempt records, supplied by the analysis service.
pub struct InvestigationOutcome {
    pub status: InvestigationStatus,
    pub result: Option<AnalysisResult>,
    pub tool_calls: Vec<ToolCallAudit>,
    pub error: Option<String>,
}

/// Inputs for a new `queued` attempt.
pub struct NewInvestigation<'a> {
    pub trace_id: &'a str,
    pub filter: &'a FailureFilter,
    pub evidence_fingerprint: &'a str,
    pub model: &'a str,
    pub prompt_version: &'a str,
}

/// Cache identity of an investigation (spec §6.4).
#[derive(Debug, Clone, Copy)]
pub struct CacheKey<'a> {
    pub trace_id: &'a str,
    pub evidence_fingerprint: &'a str,
    pub model: &'a str,
    pub prompt_version: &'a str,
}

#[async_trait]
pub trait InvestigationStore: Send + Sync {
    async fn insert(&self, record: &InvestigationRecord) -> Result<(), String>;
    async fn create_attempt(
        &self,
        input: &NewInvestigation<'_>,
    ) -> Result<InvestigationRecord, String>;
    async fn mark_running(&self, id: &str) -> Result<(), String>;
    async fn finish(&self, id: &str, outcome: &InvestigationOutcome) -> Result<(), String>;
    async fn get(&self, id: &str) -> Result<Option<InvestigationRecord>, String>;
    async fn latest(&self, trace_id: &str) -> Result<Option<InvestigationRecord>, String>;
    async fn cache_hit(&self, key: &CacheKey<'_>) -> Result<Option<InvestigationRecord>, String>;
    async fn running(&self, key: &CacheKey<'_>) -> Result<Option<InvestigationRecord>, String>;
    async fn markers(&self, trace_ids: &[String]) -> Result<Vec<InvestigationMarker>, String>;
    async fn prune_old(&self, retention_days: u32) -> Result<u64, String>;
}

/// SQLite-backed [`InvestigationStore`] sharing `data/console.db`.
pub struct SqliteInvestigationStore {
    pool: SqlitePool,
}

impl SqliteInvestigationStore {
    pub async fn new(db_url: &str) -> Result<Self, String> {
        let options = SqliteConnectOptions::from_str(db_url)
            .map_err(|error| error.to_string())?
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(5));
        // A multi-connection in-memory pool would give each connection its own
        // database, so memory databases are pinned to a single connection.
        let max_connections = if db_url.contains(":memory:") { 1 } else { 4 };
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .map_err(|error| error.to_string())?;
        // A memory database reports `memory` and ignores the pragma.
        sqlx::raw_sql("PRAGMA journal_mode=WAL;")
            .execute(&pool)
            .await
            .map_err(|error| error.to_string())?;
        sqlx::raw_sql(SCHEMA)
            .execute(&pool)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self { pool })
    }

    pub async fn memory() -> Result<Self, String> {
        Self::new("sqlite::memory:").await
    }
}

#[async_trait]
impl InvestigationStore for SqliteInvestigationStore {
    async fn insert(&self, record: &InvestigationRecord) -> Result<(), String> {
        let filter_json = encode(&record.filter)?;
        let result_json = record.result.as_ref().map(encode).transpose()?;
        let tool_calls_json = if record.tool_calls.is_empty() {
            None
        } else {
            Some(encode(&record.tool_calls)?)
        };
        sqlx::query(INSERT_SQL)
            .bind(&record.id)
            .bind(&record.trace_id)
            .bind(&filter_json)
            .bind(&record.evidence_fingerprint)
            .bind(&record.model)
            .bind(&record.prompt_version)
            .bind(record.status.as_str())
            .bind(record.verdict_category.map(|category| category.as_str()))
            .bind(record.confidence.map(|confidence| confidence.as_str()))
            .bind(result_json.as_deref())
            .bind(tool_calls_json.as_deref())
            .bind(record.error.as_deref())
            .bind(record.attempt)
            .bind(record.created_at)
            .bind(record.finished_at)
            .execute(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn create_attempt(
        &self,
        input: &NewInvestigation<'_>,
    ) -> Result<InvestigationRecord, String> {
        let id = Uuid::new_v4().to_string();
        let filter_json = encode(input.filter)?;
        let created_at = now_seconds()?;

        let mut transaction = self.pool.begin().await.map_err(|error| error.to_string())?;
        let attempt: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(attempt), 0) + 1 FROM failure_investigations \
             WHERE trace_id = ? AND evidence_fingerprint = ? AND model = ? AND prompt_version = ?",
        )
        .bind(input.trace_id)
        .bind(input.evidence_fingerprint)
        .bind(input.model)
        .bind(input.prompt_version)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        let attempt = u32::try_from(attempt)
            .map_err(|_| format!("attempt number {attempt} is out of range"))?;

        let status = InvestigationStatus::Queued;
        let no_text: Option<&str> = None;
        let no_time: Option<i64> = None;
        sqlx::query(INSERT_SQL)
            .bind(&id)
            .bind(input.trace_id)
            .bind(&filter_json)
            .bind(input.evidence_fingerprint)
            .bind(input.model)
            .bind(input.prompt_version)
            .bind(status.as_str())
            .bind(no_text)
            .bind(no_text)
            .bind(no_text)
            .bind(no_text)
            .bind(no_text)
            .bind(attempt)
            .bind(created_at)
            .bind(no_time)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;

        Ok(InvestigationRecord {
            id,
            trace_id: input.trace_id.to_string(),
            filter: input.filter.clone(),
            evidence_fingerprint: input.evidence_fingerprint.to_string(),
            model: input.model.to_string(),
            prompt_version: input.prompt_version.to_string(),
            status,
            verdict_category: None,
            confidence: None,
            result: None,
            tool_calls: Vec::new(),
            error: None,
            attempt,
            created_at,
            finished_at: None,
        })
    }

    async fn mark_running(&self, id: &str) -> Result<(), String> {
        let updated = sqlx::query(
            "UPDATE failure_investigations SET status = 'running' \
             WHERE id = ? AND status = 'queued'",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        if updated.rows_affected() == 0 {
            return Err(format!("investigation {id} is not queued"));
        }
        Ok(())
    }

    async fn finish(&self, id: &str, outcome: &InvestigationOutcome) -> Result<(), String> {
        match outcome.status {
            InvestigationStatus::Queued | InvestigationStatus::Running => {
                return Err(format!(
                    "investigation {id} cannot finish as {}",
                    outcome.status.as_str()
                ));
            }
            InvestigationStatus::Succeeded if outcome.result.is_none() => {
                return Err(format!("investigation {id} succeeded without a result"));
            }
            InvestigationStatus::Failed | InvestigationStatus::Invalid
                if outcome.result.is_some() =>
            {
                return Err(format!(
                    "investigation {id} finished as {} with a result",
                    outcome.status.as_str()
                ));
            }
            _ => {}
        }

        let verdict = outcome.result.as_ref().map(|result| &result.verdict);
        let result_json = outcome.result.as_ref().map(encode).transpose()?;
        let tool_calls_json = if outcome.tool_calls.is_empty() {
            None
        } else {
            Some(encode(&outcome.tool_calls)?)
        };
        let finished_at = now_seconds()?;

        let updated = sqlx::query(
            "UPDATE failure_investigations SET status = ?, verdict_category = ?, \
             confidence = ?, result_json = ?, tool_calls_json = ?, error = ?, finished_at = ? \
             WHERE id = ?",
        )
        .bind(outcome.status.as_str())
        .bind(verdict.map(|verdict| verdict.category.as_str()))
        .bind(verdict.map(|verdict| verdict.confidence.as_str()))
        .bind(result_json.as_deref())
        .bind(tool_calls_json.as_deref())
        .bind(outcome.error.as_deref())
        .bind(finished_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        if updated.rows_affected() == 0 {
            return Err(format!("unknown investigation {id}"));
        }
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<InvestigationRecord>, String> {
        let row = sqlx::query(SELECT_BY_ID)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        row.map(|row| decode_record(&row)).transpose()
    }

    async fn latest(&self, trace_id: &str) -> Result<Option<InvestigationRecord>, String> {
        let row = sqlx::query(SELECT_LATEST)
            .bind(trace_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        row.map(|row| decode_record(&row)).transpose()
    }

    async fn cache_hit(&self, key: &CacheKey<'_>) -> Result<Option<InvestigationRecord>, String> {
        let row = sqlx::query(SELECT_CACHE_HIT)
            .bind(key.trace_id)
            .bind(key.evidence_fingerprint)
            .bind(key.model)
            .bind(key.prompt_version)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        row.map(|row| decode_record(&row)).transpose()
    }

    async fn running(&self, key: &CacheKey<'_>) -> Result<Option<InvestigationRecord>, String> {
        let row = sqlx::query(SELECT_RUNNING)
            .bind(key.trace_id)
            .bind(key.evidence_fingerprint)
            .bind(key.model)
            .bind(key.prompt_version)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        row.map(|row| decode_record(&row)).transpose()
    }

    async fn markers(&self, trace_ids: &[String]) -> Result<Vec<InvestigationMarker>, String> {
        if trace_ids.is_empty() {
            return Ok(Vec::new());
        }
        // The only dynamic part is the placeholder count; every value is bound.
        // `rowid` is not projected, but the window function may still order by
        // it because the window runs over the inner table scan.
        let placeholders = vec!["?"; trace_ids.len()].join(", ");
        let sql = format!(
            "SELECT id, trace_id, status, verdict_category, confidence, attempt, created_at, \
             finished_at FROM (SELECT id, trace_id, status, verdict_category, confidence, \
             attempt, created_at, finished_at, ROW_NUMBER() OVER (PARTITION BY trace_id \
             ORDER BY created_at DESC, rowid DESC) AS rank FROM failure_investigations \
             WHERE trace_id IN ({placeholders})) WHERE rank = 1"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
        for trace_id in trace_ids {
            query = query.bind(trace_id);
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        rows.iter().map(decode_marker).collect()
    }

    async fn prune_old(&self, retention_days: u32) -> Result<u64, String> {
        if retention_days == 0 {
            return Ok(0);
        }
        let cutoff = now_seconds()? - i64::from(retention_days) * 86_400;
        let deleted = sqlx::query("DELETE FROM failure_investigations WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        Ok(deleted.rows_affected())
    }
}

fn now_seconds() -> Result<i64, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the unix epoch: {error}"))?
        .as_secs();
    i64::try_from(seconds).map_err(|_| "system clock is out of range".to_string())
}

fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn parse_verdict_category(text: &str) -> Result<VerdictCategory, String> {
    serde_json::from_value(serde_json::Value::String(text.to_string()))
        .map_err(|error| format!("invalid verdict_category {text:?}: {error}"))
}

fn parse_confidence(text: &str) -> Result<Confidence, String> {
    serde_json::from_value(serde_json::Value::String(text.to_string()))
        .map_err(|error| format!("invalid confidence {text:?}: {error}"))
}

fn decode_optional_category(text: Option<String>) -> Result<Option<VerdictCategory>, String> {
    text.map(|text| parse_verdict_category(&text)).transpose()
}

fn decode_optional_confidence(text: Option<String>) -> Result<Option<Confidence>, String> {
    text.map(|text| parse_confidence(&text)).transpose()
}

fn decode_record(row: &SqliteRow) -> Result<InvestigationRecord, String> {
    let filter_json: String = row.try_get("filter_json").map_err(|e| e.to_string())?;
    let status: String = row.try_get("status").map_err(|e| e.to_string())?;
    let result_json: Option<String> = row.try_get("result_json").map_err(|e| e.to_string())?;
    let tool_calls_json: Option<String> =
        row.try_get("tool_calls_json").map_err(|e| e.to_string())?;

    let filter: FailureFilter = serde_json::from_str(&filter_json)
        .map_err(|error| format!("invalid filter_json: {error}"))?;
    let result = match result_json {
        Some(text) => Some(
            serde_json::from_str::<AnalysisResult>(&text)
                .map_err(|error| format!("invalid result_json: {error}"))?,
        ),
        None => None,
    };
    let tool_calls = match tool_calls_json {
        Some(text) => serde_json::from_str::<Vec<ToolCallAudit>>(&text)
            .map_err(|error| format!("invalid tool_calls_json: {error}"))?,
        None => Vec::new(),
    };

    Ok(InvestigationRecord {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        trace_id: row.try_get("trace_id").map_err(|e| e.to_string())?,
        filter,
        evidence_fingerprint: row
            .try_get("evidence_fingerprint")
            .map_err(|e| e.to_string())?,
        model: row.try_get("model").map_err(|e| e.to_string())?,
        prompt_version: row.try_get("prompt_version").map_err(|e| e.to_string())?,
        status: InvestigationStatus::from_db(&status)?,
        verdict_category: decode_optional_category(
            row.try_get("verdict_category").map_err(|e| e.to_string())?,
        )?,
        confidence: decode_optional_confidence(
            row.try_get("confidence").map_err(|e| e.to_string())?,
        )?,
        result,
        tool_calls,
        error: row.try_get("error").map_err(|e| e.to_string())?,
        attempt: row.try_get("attempt").map_err(|e| e.to_string())?,
        created_at: row.try_get("created_at").map_err(|e| e.to_string())?,
        finished_at: row.try_get("finished_at").map_err(|e| e.to_string())?,
    })
}

fn decode_marker(row: &SqliteRow) -> Result<InvestigationMarker, String> {
    let status: String = row.try_get("status").map_err(|e| e.to_string())?;
    Ok(InvestigationMarker {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        trace_id: row.try_get("trace_id").map_err(|e| e.to_string())?,
        status: InvestigationStatus::from_db(&status)?,
        verdict_category: decode_optional_category(
            row.try_get("verdict_category").map_err(|e| e.to_string())?,
        )?,
        confidence: decode_optional_confidence(
            row.try_get("confidence").map_err(|e| e.to_string())?,
        )?,
        attempt: row.try_get("attempt").map_err(|e| e.to_string())?,
        created_at: row.try_get("created_at").map_err(|e| e.to_string())?,
        finished_at: row.try_get("finished_at").map_err(|e| e.to_string())?,
    })
}
