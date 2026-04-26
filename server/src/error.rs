use axum::{Json, http::StatusCode, response::{IntoResponse, Response}};
use platform_core::ErrorResponse;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid_api_key")]
    InvalidApiKey,
    #[error("resource_not_found")]
    NotFound,
    #[error("duplicate_name")]
    DuplicateName,
    #[error("invalid_request")]
    BadRequest(String),
    #[error("invalid_tool_arguments")]
    InvalidToolArguments,
    #[error("scheduler_capacity_exceeded")]
    SchedulerCapacityExceeded,
    #[error("tenant_concurrency_limit_exceeded")]
    TenantConcurrencyLimitExceeded,
    #[error("internal_error")]
    Internal(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
}

impl AppError {
    pub fn error_code(&self) -> &str {
        match self {
            Self::InvalidApiKey => "invalid_api_key",
            Self::NotFound => "resource_not_found",
            Self::DuplicateName => "duplicate_name",
            Self::BadRequest(_) => "invalid_request",
            Self::InvalidToolArguments => "invalid_tool_arguments",
            Self::SchedulerCapacityExceeded => "scheduler_capacity_exceeded",
            Self::TenantConcurrencyLimitExceeded => "tenant_concurrency_limit_exceeded",
            Self::Internal(_) | Self::Db(_) | Self::SerdeJson(_) => "internal_error",
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            Self::InvalidApiKey => StatusCode::UNAUTHORIZED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::DuplicateName => StatusCode::CONFLICT,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::InvalidToolArguments => StatusCode::UNPROCESSABLE_ENTITY,
            Self::SchedulerCapacityExceeded => StatusCode::SERVICE_UNAVAILABLE,
            Self::TenantConcurrencyLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) | Self::Db(_) | Self::SerdeJson(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = Json(ErrorResponse {
            error: self.error_code().to_owned(),
        });
        (status, body).into_response()
    }
}
