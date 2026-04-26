use crate::{
    app::AppState,
    auth,
    db,
    error::AppError,
    runtime,
    trace,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use platform_core::{
    AgentResponse, CreateAgentRequest, CreateApiKeyResponse, CreateTenantRequest,
    CreateTenantResponse, RunResponse, RunStatus, StartRunRequest, StartRunResponse,
    ToolResponse, TraceEventResponse,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub async fn healthz() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

pub async fn create_tenant(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateTenantRequest>,
) -> Result<Json<CreateTenantResponse>, AppError> {
    if request.name.trim().is_empty() {
        return Err(AppError::BadRequest("tenant name cannot be empty".to_owned()));
    }

    let (tenant_id, name) = db::create_tenant(&state.db, request.name.trim()).await?;
    Ok(Json(CreateTenantResponse { tenant_id, name }))
}

pub async fn create_api_key(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<CreateApiKeyResponse>, AppError> {
    let (api_key, key_prefix, key_hash) = auth::generate_api_key()?;
    db::create_api_key(&state.db, tenant_id, &key_prefix, &key_hash).await?;

    Ok(Json(CreateApiKeyResponse {
        api_key,
        warning: "This key is shown only once. Store it securely.".to_owned(),
    }))
}

pub async fn create_tool(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<platform_core::CreateToolRequest>,
) -> Result<Json<ToolResponse>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;

    if request.name.trim().is_empty() {
        return Err(AppError::BadRequest("tool name cannot be empty".to_owned()));
    }
    if !request.input_schema.is_object() || !request.mock_handler.is_object() {
        return Err(AppError::BadRequest(
            "input_schema and mock_handler must be JSON objects".to_owned(),
        ));
    }

    let tool = db::create_tool(
        &state.db,
        tenant.tenant_id,
        request.name.trim(),
        &request.input_schema,
        &request.mock_handler,
    )
    .await?;

    Ok(Json(ToolResponse {
        tool_id: tool.id,
        name: tool.name,
        input_schema: tool.input_schema,
        mock_handler: tool.mock_handler,
        created_at: tool.created_at,
    }))
}

pub async fn list_tools(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<ToolResponse>>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;
    let tools = db::list_tools(&state.db, tenant.tenant_id).await?;

    Ok(Json(
        tools
            .into_iter()
            .map(|tool| ToolResponse {
                tool_id: tool.id,
                name: tool.name,
                input_schema: tool.input_schema,
                mock_handler: tool.mock_handler,
                created_at: tool.created_at,
            })
            .collect(),
    ))
}

pub async fn delete_tool(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tool_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;
    db::delete_tool(&state.db, tenant.tenant_id, tool_id).await?;
    Ok(Json(json!({"deleted": true})))
}

pub async fn create_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Json<AgentResponse>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;

    if request.system_prompt.trim().is_empty() {
        return Err(AppError::BadRequest(
            "system_prompt cannot be empty".to_owned(),
        ));
    }

    let agent = db::create_agent(
        &state.db,
        tenant.tenant_id,
        request.name.as_deref().map(str::trim),
        request.system_prompt.trim(),
        &request.tool_ids,
        request.max_iterations,
    )
    .await?;
    let tool_ids = db::get_agent_tool_ids(&state.db, tenant.tenant_id, agent.id).await?;

    Ok(Json(AgentResponse {
        agent_id: agent.id,
        name: agent.name,
        system_prompt: agent.system_prompt,
        tool_ids,
        max_iterations: agent.max_iterations,
        created_at: agent.created_at,
    }))
}

pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<AgentResponse>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;
    let agent = db::get_agent(&state.db, tenant.tenant_id, agent_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let tool_ids = db::get_agent_tool_ids(&state.db, tenant.tenant_id, agent.id).await?;

    Ok(Json(AgentResponse {
        agent_id: agent.id,
        name: agent.name,
        system_prompt: agent.system_prompt,
        tool_ids,
        max_iterations: agent.max_iterations,
        created_at: agent.created_at,
    }))
}

pub async fn start_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(agent_id): Path<Uuid>,
    Json(request): Json<StartRunRequest>,
) -> Result<Json<StartRunResponse>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;
    let idempotency_key = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let agent = db::get_agent(&state.db, tenant.tenant_id, agent_id)
        .await?
        .ok_or(AppError::NotFound)?;

    if request.messages.is_empty() {
        return Err(AppError::BadRequest(
            "messages cannot be empty".to_owned(),
        ));
    }

    let input_messages = serde_json::to_value(&request.messages)?;
    let (run, created) = db::create_or_get_run(
        &state.db,
        tenant.tenant_id,
        agent.id,
        &input_messages,
        idempotency_key.as_deref(),
    )
    .await?;

    if created {
        trace::append_event(
            &state.db,
            tenant.tenant_id,
            run.id,
            "run_created",
            json!({"status": "pending"}),
        )
        .await?;
        runtime::schedule_run(state.clone(), tenant.tenant_id, run.id).await?;
    }

    Ok(Json(StartRunResponse {
        run_id: run.id,
        status: if created {
            RunStatus::Pending
        } else {
            match run.status.as_str() {
                "running" => RunStatus::Running,
                "success" => RunStatus::Success,
                "failed" => RunStatus::Failed,
                _ => RunStatus::Pending,
            }
        },
    }))
}

pub async fn get_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
) -> Result<Json<RunResponse>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;
    let run = db::get_run(&state.db, tenant.tenant_id, run_id)
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(Json(RunResponse {
        run_id: run.id,
        agent_id: run.agent_id,
        status: match run.status.as_str() {
            "running" => RunStatus::Running,
            "success" => RunStatus::Success,
            "failed" => RunStatus::Failed,
            _ => RunStatus::Pending,
        },
        final_answer: run.final_answer,
        error_reason: run.error_reason,
        created_at: run.created_at,
        started_at: run.started_at,
        finished_at: run.finished_at,
    }))
}

pub async fn get_trace(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
) -> Result<Json<Vec<TraceEventResponse>>, AppError> {
    let tenant = auth::authenticate(&headers, &state.db).await?;
    let traces = db::get_trace(&state.db, tenant.tenant_id, run_id).await?;

    Ok(Json(
        traces
            .into_iter()
            .map(|trace| TraceEventResponse {
                seq: trace.seq,
                event_type: trace.event_type,
                payload: trace.payload,
                created_at: trace.created_at,
            })
            .collect(),
    ))
}
