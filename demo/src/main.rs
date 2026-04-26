use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use platform_core::{
    AgentResponse, ChatMessage, CreateAgentRequest, CreateApiKeyResponse, CreateTenantRequest,
    CreateTenantResponse, CreateToolRequest, RunResponse, StartRunRequest, StartRunResponse,
    ToolResponse, TraceEventResponse,
};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

mod client;

use client::ApiClient;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    base_url: String,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Tenants(TenantsCommand),
    #[command(name = "api-keys")]
    ApiKeys(ApiKeysCommand),
    Tools(ToolsCommand),
    Agents(AgentsCommand),
    Runs(RunsCommand),
}

#[derive(Subcommand)]
enum TenantsSubcommand {
    Create { #[arg(long)] name: String },
}

#[derive(Args)]
struct TenantsCommand {
    #[command(subcommand)]
    command: TenantsSubcommand,
}

#[derive(Subcommand)]
enum ApiKeysSubcommand {
    Create { #[arg(long)] tenant_id: Uuid },
}

#[derive(Args)]
struct ApiKeysCommand {
    #[command(subcommand)]
    command: ApiKeysSubcommand,
}

#[derive(Subcommand)]
enum ToolsSubcommand {
    Create {
        #[arg(long)] api_key: String,
        #[arg(long)] name: String,
        #[arg(long)] schema_file: PathBuf,
        #[arg(long)] handler_file: PathBuf,
    },
    List {
        #[arg(long)] api_key: String,
    },
    Delete {
        #[arg(long)] api_key: String,
        #[arg(long)] tool_id: Uuid,
    },
}

#[derive(Args)]
struct ToolsCommand {
    #[command(subcommand)]
    command: ToolsSubcommand,
}

#[derive(Subcommand)]
enum AgentsSubcommand {
    Create {
        #[arg(long)] api_key: String,
        #[arg(long)] name: Option<String>,
        #[arg(long)] system_prompt: String,
        #[arg(long, required = true)] tool_id: Vec<Uuid>,
        #[arg(long, default_value_t = 10)] max_iterations: i32,
    },
    Get {
        #[arg(long)] api_key: String,
        #[arg(long)] agent_id: Uuid,
    },
}

#[derive(Args)]
struct AgentsCommand {
    #[command(subcommand)]
    command: AgentsSubcommand,
}

#[derive(Subcommand)]
enum RunsSubcommand {
    Start {
        #[arg(long)] api_key: String,
        #[arg(long)] agent_id: Uuid,
        #[arg(long)] message: String,
        #[arg(long)] idempotency_key: Option<String>,
        #[arg(long, default_value_t = false, help = "Request the invalid runtime tool-call verification flow using normal message content")] force_invalid_tool_call: bool,
    },
    Get {
        #[arg(long)] api_key: String,
        #[arg(long)] run_id: Uuid,
    },
    Wait {
        #[arg(long)] api_key: String,
        #[arg(long)] run_id: Uuid,
        #[arg(long, default_value_t = 30)] timeout_secs: u64,
        #[arg(long, default_value_t = 1)] poll_secs: u64,
    },
    Trace {
        #[arg(long)] api_key: String,
        #[arg(long)] run_id: Uuid,
    },
}

#[derive(Args)]
struct RunsCommand {
    #[command(subcommand)]
    command: RunsSubcommand,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = ApiClient::new(cli.base_url);

    match cli.command {
        Commands::Tenants(command) => handle_tenants(&client, command).await,
        Commands::ApiKeys(command) => handle_api_keys(&client, command).await,
        Commands::Tools(command) => handle_tools(&client, command).await,
        Commands::Agents(command) => handle_agents(&client, command).await,
        Commands::Runs(command) => handle_runs(&client, command).await,
    }
}

async fn handle_tenants(client: &ApiClient, command: TenantsCommand) -> Result<()> {
    match command.command {
        TenantsSubcommand::Create { name } => {
            let response: CreateTenantResponse = client
                .post("/tenants", None, &CreateTenantRequest { name }, None)
                .await?;
            print_json(&response)
        }
    }
}

async fn handle_api_keys(client: &ApiClient, command: ApiKeysCommand) -> Result<()> {
    match command.command {
        ApiKeysSubcommand::Create { tenant_id } => {
            let path = format!("/tenants/{tenant_id}/api-keys");
            let response: CreateApiKeyResponse = client.post(&path, None, &serde_json::json!({}), None).await?;
            print_json(&response)
        }
    }
}

async fn handle_tools(client: &ApiClient, command: ToolsCommand) -> Result<()> {
    match command.command {
        ToolsSubcommand::Create {
            api_key,
            name,
            schema_file,
            handler_file,
        } => {
            let input_schema = read_json_file(&schema_file)?;
            let mock_handler = read_json_file(&handler_file)?;
            let request = CreateToolRequest {
                name,
                input_schema,
                mock_handler,
            };
            let response: ToolResponse = client.post("/tools", Some(&api_key), &request, None).await?;
            print_json(&response)
        }
        ToolsSubcommand::List { api_key } => {
            let response: Vec<ToolResponse> = client.get("/tools", Some(&api_key)).await?;
            print_json(&response)
        }
        ToolsSubcommand::Delete { api_key, tool_id } => {
            let path = format!("/tools/{tool_id}");
            let response: Value = client.delete(&path, Some(&api_key)).await?;
            print_json(&response)
        }
    }
}

async fn handle_agents(client: &ApiClient, command: AgentsCommand) -> Result<()> {
    match command.command {
        AgentsSubcommand::Create {
            api_key,
            name,
            system_prompt,
            tool_id,
            max_iterations,
        } => {
            let request = CreateAgentRequest {
                name,
                system_prompt,
                tool_ids: tool_id,
                max_iterations,
            };
            let response: AgentResponse = client.post("/agents", Some(&api_key), &request, None).await?;
            print_json(&response)
        }
        AgentsSubcommand::Get { api_key, agent_id } => {
            let path = format!("/agents/{agent_id}");
            let response: AgentResponse = client.get(&path, Some(&api_key)).await?;
            print_json(&response)
        }
    }
}

async fn handle_runs(client: &ApiClient, command: RunsCommand) -> Result<()> {
    match command.command {
        RunsSubcommand::Start {
            api_key,
            agent_id,
            message,
            idempotency_key,
            force_invalid_tool_call,
        } => {
            let path = format!("/agents/{agent_id}/run");
            let content = if force_invalid_tool_call {
                format!("{} {}", message, platform_core::INVALID_TOOL_CALL_SENTINEL)
            } else {
                message
            };
            let request = StartRunRequest {
                messages: vec![ChatMessage {
                    role: "user".to_owned(),
                    content,
                }],
            };
            let response: StartRunResponse = client
                .post(&path, Some(&api_key), &request, idempotency_key.as_deref())
                .await?;
            print_json(&response)
        }
        RunsSubcommand::Get { api_key, run_id } => {
            let path = format!("/runs/{run_id}");
            let response: RunResponse = client.get(&path, Some(&api_key)).await?;
            print_json(&response)
        }
        RunsSubcommand::Wait {
            api_key,
            run_id,
            timeout_secs,
            poll_secs,
        } => {
            let response = wait_for_run(client, &api_key, run_id, timeout_secs, poll_secs).await?;
            print_json(&response)
        }
        RunsSubcommand::Trace { api_key, run_id } => {
            let path = format!("/runs/{run_id}/trace");
            let response: Vec<TraceEventResponse> = client.get(&path, Some(&api_key)).await?;
            print_json(&response)
        }
    }
}

async fn wait_for_run(
    client: &ApiClient,
    api_key: &str,
    run_id: Uuid,
    timeout_secs: u64,
    poll_secs: u64,
) -> Result<RunResponse> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let path = format!("/runs/{run_id}");

    loop {
        let run: RunResponse = client.get(&path, Some(api_key)).await?;
        if !matches!(run.status, platform_core::RunStatus::Pending | platform_core::RunStatus::Running) {
            return Ok(run);
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for run {run_id}");
        }
        tokio::time::sleep(Duration::from_secs(poll_secs)).await;
    }
}

fn read_json_file(path: &PathBuf) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid JSON in {}", path.display()))
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
