use super::{ApiError, Grants, blocking};
use crate::config::user::Access::Operate;
use crate::projects::lifecycle::{self, Closed, Start, Stop};
use axum::{
    Extension,
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub(super) async fn start(
    Path(project): Path<String>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<Response, ApiError> {
    if !grants.allows(&project, Operate) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }
    Ok(match blocking(move || lifecycle::start(&project)).await? {
        Start::Running => StatusCode::OK.into_response(),
        Start::Unregistered => StatusCode::NOT_FOUND.into_response(),
        Start::Conflict(reason) => (StatusCode::CONFLICT, reason).into_response(),
    })
}
pub(super) async fn stop(
    Path(project): Path<String>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<StatusCode, ApiError> {
    if !grants.allows(&project, Operate) {
        return Ok(StatusCode::FORBIDDEN);
    }
    Ok(
        match blocking(move || lifecycle::stop(&project, &lifecycle::niles_in)).await? {
            Stop::Stopped => StatusCode::OK,
            Stop::NotRunning => StatusCode::NOT_FOUND,
        },
    )
}
pub(super) async fn close(
    Path((project, id)): Path<(String, String)>,
    Extension(grants): Extension<Arc<Grants>>,
) -> Result<StatusCode, ApiError> {
    if !grants.allows(&project, Operate) {
        return Ok(StatusCode::FORBIDDEN);
    }
    Ok(
        match blocking(move || lifecycle::close_worker(&project, &id, &lifecycle::niles_in)).await?
        {
            Closed::Closed => StatusCode::OK,
            Closed::NotFound => StatusCode::NOT_FOUND,
        },
    )
}
