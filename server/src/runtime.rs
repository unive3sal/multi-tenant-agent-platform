use crate::{app::AppState, db, error::AppError, trace};
use jsonschema::JSONSchema;
use platform_core::{ChatMessage, INVALID_TOOL_CALL_SENTINEL, RunStatus};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{error, info};
use uuid::Uuid;

pub async fn schedule_run(
    state: Arc<AppState>,
    tenant_id: Uuid,
    run_id: Uuid,
) -> Result<(), AppError> {
    let global_permit = match state.global_semaphore.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            fail_run_before_start(
                &state,
                tenant_id,
                run_id,
                "scheduler_capacity_exceeded",
                json!({"decision": "rejected", "reason": "scheduler_capacity_exceeded"}),
            )
            .await?;
            return Err(AppError::SchedulerCapacityExceeded);
        }
    };
    let tenant_semaphore = get_tenant_semaphore(&state, tenant_id).await;
    let tenant_permit = match tenant_semaphore.try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            fail_run_before_start(
                &state,
                tenant_id,
                run_id,
                "tenant_concurrency_limit_exceeded",
                json!({"decision": "rejected", "reason": "tenant_concurrency_limit_exceeded"}),
            )
            .await?;
            return Err(AppError::TenantConcurrencyLimitExceeded);
        }
    };

    let state_for_task = state.clone();

    tokio::spawn(async move {
        if let Err(err) = execute_run(state_for_task, tenant_id, run_id, global_permit, tenant_permit).await {
            error!(tenant_id = %tenant_id, run_id = %run_id, error = %err, "run execution failed");
        }
    });

    Ok(())
}

async fn fail_run_before_start(
    state: &Arc<AppState>,
    tenant_id: Uuid,
    run_id: Uuid,
    error_reason: &str,
    scheduler_payload: Value,
) -> Result<(), AppError> {
    db::mark_run_failed(&state.db, tenant_id, run_id, error_reason).await?;
    trace::append_event(&state.db, tenant_id, run_id, "scheduler_decision", scheduler_payload).await?;
    trace::append_event(
        &state.db,
        tenant_id,
        run_id,
        "run_end",
        json!({"status": "failed", "error_reason": error_reason}),
    )
    .await?;
    Ok(())
}

async fn execute_run(
    state: Arc<AppState>,
    tenant_id: Uuid,
    run_id: Uuid,
    _global_permit: OwnedSemaphorePermit,
    _tenant_permit: OwnedSemaphorePermit,
) -> Result<(), AppError> {
    let can_start = db::begin_run_execution(&state.db, tenant_id, run_id).await?;
    if !can_start {
        return Ok(());
    }

    trace::append_event(
        &state.db,
        tenant_id,
        run_id,
        "scheduler_decision",
        json!({"decision": "accepted"}),
    )
    .await?;
    trace::append_event(
        &state.db,
        tenant_id,
        run_id,
        "run_started",
        json!({"status": "running"}),
    )
    .await?;

    let run = db::get_run(&state.db, tenant_id, run_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let agent = db::get_agent(&state.db, tenant_id, run.agent_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let bound_tools = db::list_agent_tools(&state.db, tenant_id, agent.id).await?;
    let mut messages: Vec<ChatMessage> = serde_json::from_value(run.input_messages.clone())?;
    let force_invalid_tool_call = messages.iter().any(message_requests_invalid_tool_call);

    let mut tool_result_count = 0;
    let mut invalid_trigger_consumed = false;

    for iteration in 1..=agent.max_iterations {
        let llm_decision = mock_llm_decision(
            &state,
            tenant_id,
            agent.id,
            &bound_tools,
            tool_result_count,
            force_invalid_tool_call && !invalid_trigger_consumed,
        )
        .await?;
        invalid_trigger_consumed |= force_invalid_tool_call;

        trace::append_event(
            &state.db,
            tenant_id,
            run_id,
            "llm_call",
            json!({
                "iteration": iteration,
                "decision": llm_decision.trace_payload,
            }),
        )
        .await?;

        match llm_decision.kind {
            LlmDecisionKind::FinalAnswer(answer) => {
                db::finalize_run(&state.db, tenant_id, run_id, RunStatus::Success, Some(&answer), None)
                    .await?;
                trace::append_event(
                    &state.db,
                    tenant_id,
                    run_id,
                    "run_end",
                    json!({"status": "success", "final_answer": answer}),
                )
                .await?;
                info!(tenant_id = %tenant_id, run_id = %run_id, agent_id = %agent.id, status = "success", "run completed");
                return Ok(());
            }
            LlmDecisionKind::ToolCall { tool_id, arguments } => {
                let tool = match authorize_tool_call(&state, tenant_id, agent.id, tool_id).await {
                    Ok(tool) => tool,
                    Err(err) => {
                        let error_reason = normalize_error_reason(&err);
                        db::finalize_run(&state.db, tenant_id, run_id, RunStatus::Failed, None, Some(error_reason))
                            .await?;
                        trace::append_event(
                            &state.db,
                            tenant_id,
                            run_id,
                            "run_end",
                            json!({"status": "failed", "error_reason": error_reason}),
                        )
                        .await?;
                        return Err(err);
                    }
                };
                if let Err(err) = validate_tool_arguments(&tool.input_schema, &arguments) {
                    let error_reason = normalize_error_reason(&err);
                    db::finalize_run(&state.db, tenant_id, run_id, RunStatus::Failed, None, Some(error_reason))
                        .await?;
                    trace::append_event(
                        &state.db,
                        tenant_id,
                        run_id,
                        "run_end",
                        json!({"status": "failed", "error_reason": error_reason}),
                    )
                    .await?;
                    return Err(err);
                }
                let result = execute_mock_handler(&tool.mock_handler)?;
                trace::append_event(
                    &state.db,
                    tenant_id,
                    run_id,
                    "tool_exec",
                    json!({
                        "tool_id": tool.id,
                        "tool_name": tool.name,
                        "arguments": arguments,
                        "result": result,
                    }),
                )
                .await?;
                messages.push(ChatMessage {
                    role: "tool".to_owned(),
                    content: serde_json::to_string(&result)?,
                });
                tool_result_count += 1;
            }
        }
    }

    db::finalize_run(
        &state.db,
        tenant_id,
        run_id,
        RunStatus::Failed,
        None,
        Some("max_iterations_exceeded"),
    )
    .await?;
    trace::append_event(
        &state.db,
        tenant_id,
        run_id,
        "run_end",
        json!({"status": "failed", "error_reason": "max_iterations_exceeded"}),
    )
    .await?;
    Ok(())
}

async fn get_tenant_semaphore(state: &Arc<AppState>, tenant_id: Uuid) -> Arc<Semaphore> {
    let mut semaphores = state.tenant_semaphores.lock().await;
    semaphores
        .entry(tenant_id)
        .or_insert_with(|| Arc::new(Semaphore::new(state.config.per_tenant_max_concurrency)))
        .clone()
}

struct LlmDecision {
    kind: LlmDecisionKind,
    trace_payload: Value,
}

enum LlmDecisionKind {
    FinalAnswer(String),
    ToolCall { tool_id: Uuid, arguments: Value },
}

async fn mock_llm_decision(
    state: &Arc<AppState>,
    tenant_id: Uuid,
    agent_id: Uuid,
    bound_tools: &[db::ToolRecord],
    tool_result_count: i32,
    force_invalid_tool_call: bool,
) -> Result<LlmDecision, AppError> {
    if force_invalid_tool_call {
        if let Some(tool) = db::find_unbound_active_tool(&state.db, tenant_id, agent_id).await? {
            return Ok(LlmDecision {
                trace_payload: json!({"type": "tool_call", "tool_id": tool.id, "forced": true}),
                kind: LlmDecisionKind::ToolCall {
                    tool_id: tool.id,
                    arguments: json!({"query": "forced invalid lookup"}),
                },
            });
        }

        return Ok(LlmDecision {
            trace_payload: json!({"type": "tool_call", "tool_id": Uuid::nil(), "forced": true}),
            kind: LlmDecisionKind::ToolCall {
                tool_id: Uuid::nil(),
                arguments: json!({"query": "forced invalid lookup"}),
            },
        });
    }

    if tool_result_count < 2 {
        let tool = bound_tools.first().ok_or_else(|| {
            AppError::Internal("agent has no bound tools for deterministic loop".to_owned())
        })?;
        return Ok(LlmDecision {
            trace_payload: json!({"type": "tool_call", "tool_id": tool.id}),
            kind: LlmDecisionKind::ToolCall {
                tool_id: tool.id,
                arguments: json!({"query": format!("iteration-{tool_result_count}")}),
            },
        });
    }

    Ok(LlmDecision {
        trace_payload: json!({"type": "final_answer"}),
        kind: LlmDecisionKind::FinalAnswer("Deterministic mock final answer.".to_owned()),
    })
}

async fn authorize_tool_call(
    state: &Arc<AppState>,
    tenant_id: Uuid,
    agent_id: Uuid,
    tool_id: Uuid,
) -> Result<db::ToolRecord, AppError> {
    let tool = db::get_tool(&state.db, tenant_id, tool_id).await?;

    let Some(tool) = tool else {
        return Err(AppError::BadRequest("tool_not_allowed_for_agent".to_owned()));
    };

    if tool.deleted_at.is_some() {
        return Err(AppError::BadRequest("tool_deleted".to_owned()));
    }

    let allowed_tool_ids = db::get_agent_tool_ids(&state.db, tenant_id, agent_id).await?;
    if !allowed_tool_ids.contains(&tool_id) {
        return Err(AppError::BadRequest("tool_not_allowed_for_agent".to_owned()));
    }

    Ok(tool)
}

fn message_requests_invalid_tool_call(message: &ChatMessage) -> bool {
    message.role == "user" && message.content.contains(INVALID_TOOL_CALL_SENTINEL)
}

fn validate_tool_arguments(input_schema: &Value, arguments: &Value) -> Result<(), AppError> {
    let compiled = JSONSchema::compile(input_schema)
        .map_err(|_| AppError::BadRequest("invalid input schema".to_owned()))?;
    if compiled.is_valid(arguments) {
        Ok(())
    } else {
        Err(AppError::InvalidToolArguments)
    }
}

fn normalize_error_reason(err: &AppError) -> &str {
    match err {
        AppError::BadRequest(reason) => reason.as_str(),
        AppError::InvalidToolArguments => "invalid_tool_arguments",
        AppError::SchedulerCapacityExceeded => "scheduler_capacity_exceeded",
        AppError::TenantConcurrencyLimitExceeded => "tenant_concurrency_limit_exceeded",
        AppError::NotFound => "resource_not_found",
        _ => "internal_error",
    }
}

fn execute_mock_handler(mock_handler: &Value) -> Result<Value, AppError> {
    match mock_handler.get("type").and_then(Value::as_str) {
        Some("static") => mock_handler
            .get("response")
            .cloned()
            .ok_or_else(|| AppError::BadRequest("mock_handler.response is required".to_owned())),
        _ => Err(AppError::BadRequest(
            "only static mock handlers are supported".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{execute_mock_handler, normalize_error_reason, validate_tool_arguments};
    use serde_json::json;

    #[test]
    fn validate_tool_arguments_returns_error_for_schema_violation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });
        let arguments = json!({"wrong": 1});

        let err = validate_tool_arguments(&schema, &arguments).unwrap_err();

        assert_eq!(normalize_error_reason(&err), "invalid_tool_arguments");
    }

    #[test]
    fn execute_mock_handler_returns_static_response() {
        let handler = json!({
            "type": "static",
            "response": {"result": "mock search result"}
        });

        let response = execute_mock_handler(&handler).unwrap();

        assert_eq!(response, json!({"result": "mock search result"}));
    }
}
