use chrono::{DateTime, Utc};
use platform_core::RunStatus;
use serde_json::Value;
use sqlx::{PgPool, Row, types::Json};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct ApiKeyRecord {
    pub tenant_id: Uuid,
    pub key_hash: String,
}

#[derive(Debug, Clone)]
pub struct ToolRecord {
    pub id: Uuid,
    pub name: String,
    pub input_schema: Value,
    pub mock_handler: Value,
    pub created_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct AgentRecord {
    pub id: Uuid,
    pub name: Option<String>,
    pub system_prompt: String,
    pub max_iterations: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RunRecord {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub status: String,
    pub input_messages: Value,
    pub final_answer: Option<String>,
    pub error_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TraceRecord {
    pub seq: i32,
    pub event_type: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

pub async fn create_tenant(db: &PgPool, name: &str) -> Result<(Uuid, String), AppError> {
    let row = sqlx::query(
        r#"
        INSERT INTO tenants (name)
        VALUES ($1)
        RETURNING id, name
        "#,
    )
    .bind(name)
    .fetch_one(db)
    .await?;

    Ok((row.get("id"), row.get("name")))
}

pub async fn create_api_key(
    db: &PgPool,
    tenant_id: Uuid,
    key_prefix: &str,
    key_hash: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO api_keys (tenant_id, key_prefix, key_hash)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(tenant_id)
    .bind(key_prefix)
    .bind(key_hash)
    .execute(db)
    .await?;

    Ok(())
}

pub async fn find_active_api_key_by_prefix(
    db: &PgPool,
    key_prefix: &str,
) -> Result<Option<ApiKeyRecord>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT tenant_id, key_hash
        FROM api_keys
        WHERE key_prefix = $1
          AND revoked_at IS NULL
        "#,
    )
    .bind(key_prefix)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|row| ApiKeyRecord {
        tenant_id: row.get("tenant_id"),
        key_hash: row.get("key_hash"),
    }))
}

pub async fn create_tool(
    db: &PgPool,
    tenant_id: Uuid,
    name: &str,
    input_schema: &Value,
    mock_handler: &Value,
) -> Result<ToolRecord, AppError> {
    let result = sqlx::query(
        r#"
        INSERT INTO tools (tenant_id, name, input_schema, mock_handler)
        VALUES ($1, $2, $3, $4)
        RETURNING id, tenant_id, name, input_schema, mock_handler, created_at, deleted_at
        "#,
    )
    .bind(tenant_id)
    .bind(name)
    .bind(Json(input_schema.clone()))
    .bind(Json(mock_handler.clone()))
    .fetch_one(db)
    .await;

    match result {
        Ok(row) => Ok(tool_from_row(row)),
        Err(sqlx::Error::Database(err)) if err.constraint() == Some("uq_tools_tenant_name_active") => {
            Err(AppError::DuplicateName)
        }
        Err(err) => Err(AppError::Db(err)),
    }
}

pub async fn list_tools(db: &PgPool, tenant_id: Uuid) -> Result<Vec<ToolRecord>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, name, input_schema, mock_handler, created_at, deleted_at
        FROM tools
        WHERE tenant_id = $1
          AND deleted_at IS NULL
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(tool_from_row).collect())
}

pub async fn delete_tool(db: &PgPool, tenant_id: Uuid, tool_id: Uuid) -> Result<(), AppError> {
    let result = sqlx::query(
        r#"
        UPDATE tools
        SET deleted_at = now()
        WHERE tenant_id = $1
          AND id = $2
          AND deleted_at IS NULL
        "#,
    )
    .bind(tenant_id)
    .bind(tool_id)
    .execute(db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }

    Ok(())
}

pub async fn create_agent(
    db: &PgPool,
    tenant_id: Uuid,
    name: Option<&str>,
    system_prompt: &str,
    tool_ids: &[Uuid],
    max_iterations: i32,
) -> Result<AgentRecord, AppError> {
    let mut tx = db.begin().await?;

    let result = sqlx::query(
        r#"
        INSERT INTO agents (tenant_id, name, system_prompt, max_iterations)
        VALUES ($1, $2, $3, $4)
        RETURNING id, tenant_id, name, system_prompt, max_iterations, created_at
        "#,
    )
    .bind(tenant_id)
    .bind(name)
    .bind(system_prompt)
    .bind(max_iterations)
    .fetch_one(&mut *tx)
    .await;

    let agent = match result {
        Ok(row) => AgentRecord {
            id: row.get("id"),
            name: row.get("name"),
            system_prompt: row.get("system_prompt"),
            max_iterations: row.get("max_iterations"),
            created_at: row.get("created_at"),
        },
        Err(sqlx::Error::Database(err)) if err.constraint() == Some("uq_agents_tenant_name_active") => {
            return Err(AppError::DuplicateName);
        }
        Err(err) => return Err(AppError::Db(err)),
    };

    for tool_id in tool_ids {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM tools
                WHERE tenant_id = $1
                  AND id = $2
                  AND deleted_at IS NULL
            )
            "#,
        )
        .bind(tenant_id)
        .bind(tool_id)
        .fetch_one(&mut *tx)
        .await?;

        if !exists {
            return Err(AppError::NotFound);
        }

        sqlx::query(
            r#"
            INSERT INTO agent_tools (tenant_id, agent_id, tool_id)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(tenant_id)
        .bind(agent.id)
        .bind(tool_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(agent)
}

pub async fn get_agent(db: &PgPool, tenant_id: Uuid, agent_id: Uuid) -> Result<Option<AgentRecord>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, tenant_id, name, system_prompt, max_iterations, created_at
        FROM agents
        WHERE tenant_id = $1
          AND id = $2
          AND deleted_at IS NULL
        "#,
    )
    .bind(tenant_id)
    .bind(agent_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(|row| AgentRecord {
        id: row.get("id"),
        name: row.get("name"),
        system_prompt: row.get("system_prompt"),
        max_iterations: row.get("max_iterations"),
        created_at: row.get("created_at"),
    }))
}

pub async fn get_agent_tool_ids(
    db: &PgPool,
    tenant_id: Uuid,
    agent_id: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT tool_id
        FROM agent_tools
        WHERE tenant_id = $1
          AND agent_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(agent_id)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(|row| row.get("tool_id")).collect())
}

pub async fn create_or_get_run(
    db: &PgPool,
    tenant_id: Uuid,
    agent_id: Uuid,
    input_messages: &Value,
    idempotency_key: Option<&str>,
) -> Result<(RunRecord, bool), AppError> {
    let mut tx = db.begin().await?;

    let inserted = sqlx::query(
        r#"
        INSERT INTO runs (tenant_id, agent_id, input_messages, idempotency_key)
        VALUES ($1, $2, $3, $4)
        RETURNING id, tenant_id, agent_id, status, input_messages, final_answer, error_reason,
                  idempotency_key, max_retries, retry_count, created_at, started_at, finished_at
        "#,
    )
    .bind(tenant_id)
    .bind(agent_id)
    .bind(Json(input_messages.clone()))
    .bind(idempotency_key)
    .fetch_one(&mut *tx)
    .await;

    match inserted {
        Ok(row) => {
            let run = run_from_row(row);
            tx.commit().await?;
            Ok((run, true))
        }
        Err(sqlx::Error::Database(err)) if err.constraint() == Some("uq_runs_idempotency") => {
            let row = sqlx::query(
                r#"
                SELECT id, tenant_id, agent_id, status, input_messages, final_answer, error_reason,
                       idempotency_key, max_retries, retry_count, created_at, started_at, finished_at
                FROM runs
                WHERE tenant_id = $1
                  AND agent_id = $2
                  AND idempotency_key = $3
                "#,
            )
            .bind(tenant_id)
            .bind(agent_id)
            .bind(idempotency_key)
            .fetch_one(&mut *tx)
            .await?;
            let run = run_from_row(row);
            tx.commit().await?;
            Ok((run, false))
        }
        Err(err) => Err(AppError::Db(err)),
    }
}

pub async fn mark_run_failed(
    db: &PgPool,
    tenant_id: Uuid,
    run_id: Uuid,
    error_reason: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        UPDATE runs
        SET status = 'failed',
            error_reason = $3,
            finished_at = now()
        WHERE tenant_id = $1
          AND id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(run_id)
    .bind(error_reason)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn begin_run_execution(
    db: &PgPool,
    tenant_id: Uuid,
    run_id: Uuid,
) -> Result<bool, AppError> {
    let result = sqlx::query(
        r#"
        UPDATE runs
        SET status = 'running',
            started_at = now()
        WHERE id = $1
          AND tenant_id = $2
          AND status = 'pending'
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .execute(db)
    .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn get_run(db: &PgPool, tenant_id: Uuid, run_id: Uuid) -> Result<Option<RunRecord>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, tenant_id, agent_id, status, input_messages, final_answer, error_reason,
               idempotency_key, max_retries, retry_count, created_at, started_at, finished_at
        FROM runs
        WHERE tenant_id = $1
          AND id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(run_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(run_from_row))
}

pub async fn get_trace(db: &PgPool, tenant_id: Uuid, run_id: Uuid) -> Result<Vec<TraceRecord>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT seq, type, payload, created_at
        FROM run_traces
        WHERE tenant_id = $1
          AND run_id = $2
        ORDER BY seq ASC
        "#,
    )
    .bind(tenant_id)
    .bind(run_id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| TraceRecord {
            seq: row.get("seq"),
            event_type: row.get("type"),
            payload: row.get::<Json<Value>, _>("payload").0,
            created_at: row.get("created_at"),
        })
        .collect())
}

pub async fn get_tool(db: &PgPool, tenant_id: Uuid, tool_id: Uuid) -> Result<Option<ToolRecord>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, tenant_id, name, input_schema, mock_handler, created_at, deleted_at
        FROM tools
        WHERE tenant_id = $1
          AND id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(tool_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(tool_from_row))
}

pub async fn list_agent_tools(db: &PgPool, tenant_id: Uuid, agent_id: Uuid) -> Result<Vec<ToolRecord>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT t.id, t.tenant_id, t.name, t.input_schema, t.mock_handler, t.created_at, t.deleted_at
        FROM agent_tools at
        JOIN tools t
          ON t.tenant_id = at.tenant_id
         AND t.id = at.tool_id
        WHERE at.tenant_id = $1
          AND at.agent_id = $2
        ORDER BY at.created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(agent_id)
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(tool_from_row).collect())
}

pub async fn find_unbound_active_tool(
    db: &PgPool,
    tenant_id: Uuid,
    agent_id: Uuid,
) -> Result<Option<ToolRecord>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT t.id, t.tenant_id, t.name, t.input_schema, t.mock_handler, t.created_at, t.deleted_at
        FROM tools t
        WHERE t.tenant_id = $1
          AND t.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1
              FROM agent_tools at
              WHERE at.tenant_id = t.tenant_id
                AND at.agent_id = $2
                AND at.tool_id = t.id
          )
        ORDER BY t.created_at ASC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(agent_id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(tool_from_row))
}

pub async fn finalize_run(
    db: &PgPool,
    tenant_id: Uuid,
    run_id: Uuid,
    status: RunStatus,
    final_answer: Option<&str>,
    error_reason: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        UPDATE runs
        SET status = $3,
            final_answer = $4,
            error_reason = $5,
            finished_at = now()
        WHERE tenant_id = $1
          AND id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(run_id)
    .bind(status.as_str())
    .bind(final_answer)
    .bind(error_reason)
    .execute(db)
    .await?;
    Ok(())
}

fn tool_from_row(row: sqlx::postgres::PgRow) -> ToolRecord {
    ToolRecord {
        id: row.get("id"),
        name: row.get("name"),
        input_schema: row.get::<Json<Value>, _>("input_schema").0,
        mock_handler: row.get::<Json<Value>, _>("mock_handler").0,
        created_at: row.get("created_at"),
        deleted_at: row.get("deleted_at"),
    }
}

fn run_from_row(row: sqlx::postgres::PgRow) -> RunRecord {
    RunRecord {
        id: row.get("id"),
        agent_id: row.get("agent_id"),
        status: row.get("status"),
        input_messages: row.get::<Json<Value>, _>("input_messages").0,
        final_answer: row.get("final_answer"),
        error_reason: row.get("error_reason"),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    }
}
