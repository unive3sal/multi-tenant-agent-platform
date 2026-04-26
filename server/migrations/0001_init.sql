CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE tenants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    key_prefix TEXT NOT NULL,
    key_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ NULL
);

CREATE UNIQUE INDEX uq_api_keys_key_prefix
ON api_keys (key_prefix);

CREATE INDEX idx_api_keys_tenant_id
ON api_keys (tenant_id);

CREATE INDEX idx_api_keys_active_prefix
ON api_keys (key_prefix)
WHERE revoked_at IS NULL;

CREATE TABLE tools (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    input_schema JSONB NOT NULL,
    mock_handler JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ NULL,
    UNIQUE (tenant_id, id),
    CHECK (jsonb_typeof(input_schema) = 'object'),
    CHECK (jsonb_typeof(mock_handler) = 'object')
);

CREATE UNIQUE INDEX uq_tools_tenant_name_active
ON tools (tenant_id, name)
WHERE deleted_at IS NULL;

CREATE INDEX idx_tools_tenant_active
ON tools (tenant_id)
WHERE deleted_at IS NULL;

CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name TEXT NULL,
    system_prompt TEXT NOT NULL,
    max_iterations INTEGER NOT NULL DEFAULT 10,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ NULL,
    UNIQUE (tenant_id, id),
    CHECK (max_iterations > 0),
    CHECK (max_iterations <= 50)
);

CREATE UNIQUE INDEX uq_agents_tenant_name_active
ON agents (tenant_id, name)
WHERE deleted_at IS NULL AND name IS NOT NULL;

CREATE INDEX idx_agents_tenant_active
ON agents (tenant_id)
WHERE deleted_at IS NULL;

CREATE TABLE agent_tools (
    tenant_id UUID NOT NULL,
    agent_id UUID NOT NULL,
    tool_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, agent_id, tool_id),
    FOREIGN KEY (tenant_id, agent_id)
        REFERENCES agents(tenant_id, id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, tool_id)
        REFERENCES tools(tenant_id, id)
        ON DELETE CASCADE
);

CREATE TABLE runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    agent_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    input_messages JSONB NOT NULL,
    final_answer TEXT NULL,
    error_reason TEXT NULL,
    idempotency_key TEXT NULL,
    max_retries INTEGER NOT NULL DEFAULT 3,
    retry_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL,
    UNIQUE (tenant_id, id),
    FOREIGN KEY (tenant_id, agent_id)
        REFERENCES agents(tenant_id, id)
        ON DELETE RESTRICT,
    CHECK (jsonb_typeof(input_messages) = 'array'),
    CHECK (status IN ('pending', 'running', 'success', 'failed')),
    CHECK (max_retries >= 0),
    CHECK (max_retries <= 10),
    CHECK (retry_count >= 0)
);

CREATE UNIQUE INDEX uq_runs_idempotency
ON runs (tenant_id, agent_id, idempotency_key)
WHERE idempotency_key IS NOT NULL;

CREATE INDEX idx_runs_tenant
ON runs (tenant_id);

CREATE INDEX idx_runs_tenant_agent
ON runs (tenant_id, agent_id);

CREATE INDEX idx_runs_tenant_status
ON runs (tenant_id, status);

CREATE TABLE run_traces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    run_id UUID NOT NULL,
    seq INTEGER NOT NULL,
    type TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES runs(tenant_id, id)
        ON DELETE CASCADE,
    UNIQUE (tenant_id, run_id, seq),
    CHECK (seq > 0),
    CHECK (
        type IN (
            'run_created',
            'scheduler_decision',
            'run_started',
            'llm_call',
            'tool_exec',
            'retry',
            'run_end'
        )
    ),
    CHECK (jsonb_typeof(payload) = 'object')
);

CREATE INDEX idx_run_traces_tenant_run_seq
ON run_traces (tenant_id, run_id, seq);
