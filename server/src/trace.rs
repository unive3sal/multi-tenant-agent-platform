use crate::error::AppError;
use serde_json::Value;
use sqlx::{Executor, Postgres, types::Json};
use uuid::Uuid;

pub async fn append_event<'a, E>(
    executor: E,
    tenant_id: Uuid,
    run_id: Uuid,
    event_type: &str,
    payload: Value,
) -> Result<(), AppError>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO run_traces (tenant_id, run_id, seq, type, payload)
        SELECT $1, $2, COALESCE(MAX(seq), 0) + 1, $3, $4
        FROM run_traces
        WHERE tenant_id = $1 AND run_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(run_id)
    .bind(event_type)
    .bind(Json(payload))
    .execute(executor)
    .await?;

    Ok(())
}
