use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use daymark_domain::Error;
use daymark_persistence::StoreError;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub &'static str, pub String);
pub type Result<T> = std::result::Result<T, ApiError>;
impl ApiError {
    pub fn bad() -> Self {
        Self(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "Check the request fields and input bounds.".into(),
        )
    }
    pub fn unauthorized() -> Self {
        Self(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Authentication required or credentials invalid.".into(),
        )
    }
    pub fn forbidden() -> Self {
        Self(
            StatusCode::FORBIDDEN,
            "forbidden",
            "This operation is not permitted.".into(),
        )
    }
    pub fn missing() -> Self {
        Self(
            StatusCode::NOT_FOUND,
            "not_found",
            "The requested record was not found.".into(),
        )
    }
    pub fn busy() -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Service unavailable; reload before retrying.".into(),
        )
    }
    pub fn limited() -> Self {
        Self(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts; try again later.".into(),
        )
    }
    pub fn conflict() -> Self {
        Self(
            StatusCode::CONFLICT,
            "account_unavailable",
            "Account could not be created.".into(),
        )
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.2)
    }
}
impl std::error::Error for ApiError {}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response =
            (self.0, Json(json!({"error": self.1, "message": self.2}))).into_response();
        if matches!(
            self.0,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
        ) {
            response
                .headers_mut()
                .insert("retry-after", "60".parse().unwrap());
        }
        response
    }
}
impl From<rusqlite::Error> for ApiError {
    fn from(_: rusqlite::Error) -> Self {
        tracing::warn!(event = "storage_failure");
        Self::busy()
    }
}
impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Domain(e) => e.into(),
            StoreError::StaleCalendar => Self(
                StatusCode::CONFLICT,
                "stale_calendar",
                "The calendar changed; reload before submitting.".into(),
            ),
            _ => {
                tracing::warn!(event = "ledger_storage_failure");
                Self::busy()
            }
        }
    }
}
impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        let (status, code) = match error {
            Error::NotFound => return Self::missing(),
            Error::StaleRevision => (StatusCode::CONFLICT, "stale_revision"),
            Error::AlreadyExists => (StatusCode::CONFLICT, "source_unavailable"),
            Error::ReferencedHoliday => (StatusCode::CONFLICT, "holiday_referenced"),
            Error::NegativeBalance { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "negative_balance"),
            _ => (StatusCode::UNPROCESSABLE_ENTITY, "domain_validation"),
        };
        Self(status, code, error.to_string())
    }
}
