use super::{Failure, history::History, worker::Context};
use crate::{
    image::Evidence,
    json::{self, Value},
    tools::Output,
    workspace::{Budget, Error as WorkspaceError, Workspace},
};
use std::time::{Duration, Instant};

pub(super) fn execute(
    history: &History,
    workspace: &Workspace,
    path: Option<&str>,
    image_id: Option<&str>,
    context: &Context,
) -> Result<(Output, Option<Evidence>), Failure> {
    if !history.can_view_images() {
        return Ok((
            Output::error("view_image is unavailable for this model or session"),
            None,
        ));
    }
    let images = history.images()?;
    let evidence = if let Some(path) = path {
        let budget = Budget {
            cancelled: &context.cancelled,
            deadline: Instant::now() + Duration::from_secs(10),
        };
        let (bytes, display) = match workspace.read_image_bytes(path, &budget) {
            Ok(result) => result,
            Err(error) => return Ok((Output::error(&workspace_error(error)), None)),
        };
        context.check()?;
        match images.capture(&bytes, &display) {
            Ok(evidence) => evidence,
            Err(error) => return Ok((Output::error(error.message()), None)),
        }
    } else if let Some(id) = image_id {
        match images.load_id(id) {
            Ok(evidence) => evidence,
            Err(error) => return Ok((Output::error(error.message()), None)),
        }
    } else {
        return Ok((Output::error("view_image requires path or image_id"), None));
    };
    context.check()?;
    let summary = evidence.description();
    let output = Output::success(
        json::object([
            ("ok", Value::Bool(true)),
            ("path", Value::String(evidence.path.clone())),
            ("format", Value::String("png".into())),
            ("width", Value::Number(evidence.width.to_string())),
            ("height", Value::Number(evidence.height.to_string())),
            ("bytes", Value::Number(evidence.bytes.to_string())),
            ("image_id", Value::String(evidence.id.clone())),
            ("detail", Value::String("high".into())),
        ]),
        summary,
        false,
    );
    Ok((output, Some(evidence)))
}

fn workspace_error(error: WorkspaceError) -> String {
    match error {
        WorkspaceError::Size => crate::image::Error::Limit.message().into(),
        WorkspaceError::Missing => "image file does not exist".into(),
        WorkspaceError::Changed => "image changed during capture; retry after it is stable".into(),
        _ => error.to_string(),
    }
}
