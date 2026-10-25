use super::{ApiError, blocking};
use crate::projects::lifecycle::{self, Closed, Start, Stop};
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub(super) async fn start(Path(project): Path<String>) -> Result<Response, ApiError> {
    Ok(match blocking(move || lifecycle::start(&project)).await? {
        Start::Running => StatusCode::OK.into_response(),
        Start::Unregistered => StatusCode::NOT_FOUND.into_response(),
        // Fixed text distinguishes operator-fixable conflicts without exposing filesystem detail.
        Start::DirectoryMissing => {
            (StatusCode::CONFLICT, "Project directory missing").into_response()
        }
        Start::NoManifest => (
            StatusCode::CONFLICT,
            "No saved manifest; start once at the desk",
        )
            .into_response(),
        Start::NameTaken => (
            StatusCode::CONFLICT,
            "Tmux session name held by a foreign session",
        )
            .into_response(),
    })
}
pub(super) async fn stop(Path(project): Path<String>) -> Result<StatusCode, ApiError> {
    Ok(
        match blocking(move || lifecycle::stop(&project, &lifecycle::niles_in)).await? {
            Stop::Stopped => StatusCode::OK,
            Stop::NotRunning => StatusCode::NOT_FOUND,
        },
    )
}
pub(super) async fn close(
    Path((project, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    Ok(
        match blocking(move || lifecycle::close_worker(&project, &id, &lifecycle::niles_in)).await?
        {
            Closed::Closed => StatusCode::OK,
            Closed::NotFound => StatusCode::NOT_FOUND,
        },
    )
}
