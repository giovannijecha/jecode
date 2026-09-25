use super::{ContentKind, Error, Limits, Progress, Request, Response, ResponseStream};
use crate::http;
use std::ops::ControlFlow;

/// Contains authorization bytes. Pass only to authenticated TLS for chatgpt.com.
/// No redirects, logging, credential discovery or network I/O occurs here.
pub fn encode_http(
    request: &Request,
    access_token: &str,
    account_id: &str,
) -> Result<Vec<u8>, Error> {
    if access_token.is_empty()
        || access_token.len() > 8192
        || !access_token.bytes().all(|b| (33..=126).contains(&b))
        || !super::identifier(account_id)
    {
        return Err(Error::InvalidRequest);
    }
    let body = request.encode(8 * 1024 * 1024)?;
    http::post_json(
        "chatgpt.com",
        "/backend-api/codex/responses",
        &[
            ("Authorization", &format!("Bearer {access_token}")),
            ("ChatGPT-Account-Id", account_id),
            ("OpenAI-Beta", "responses=experimental"),
            ("Accept", "text/event-stream"),
        ],
        &body,
        8 * 1024 * 1024 + 32768,
    )
    .map_err(Error::Http)
}

/// HTTP body framing followed by SSE and account event validation.
/// Input must come from an authenticated transport, supplied by `client::Client`.
pub struct HttpResponseStream {
    http: http::Decoder,
    model: ResponseStream,
    error: Option<Error>,
    completed: bool,
    stream_started: bool,
    response_status: Option<u16>,
}
impl Default for HttpResponseStream {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}
impl HttpResponseStream {
    pub fn new(limits: Limits) -> Self {
        Self {
            http: http::Decoder::new(http::Limits {
                body_bytes: limits.wire_bytes,
                ..Default::default()
            }),
            model: ResponseStream::new(limits),
            error: None,
            completed: false,
            stream_started: false,
            response_status: None,
        }
    }
    pub fn is_finished(&self) -> bool {
        self.completed || self.error.is_some()
    }
    pub fn stream_started(&self) -> bool {
        self.stream_started
    }
    pub fn response_status(&self) -> Option<u16> {
        self.response_status
    }
    pub fn stream_events(&self) -> u32 {
        self.model.events_seen()
    }
    pub fn push(
        &mut self,
        bytes: &[u8],
        mut progress: impl FnMut(Progress<'_>) -> ControlFlow<()>,
    ) -> Result<(), Error> {
        if self.is_finished() {
            return Err(Error::Closed);
        }
        let model = &mut self.model;
        let stream_started = &mut self.stream_started;
        let response_status = &mut self.response_status;
        let mut failure = None;
        let mut completed = false;
        let result = self.http.push(bytes, |event| {
            let result = match event {
                http::Event::Head(head) if head.status != 200 => {
                    *response_status = Some(head.status);
                    Err(Error::HttpStatus(head.status))
                }
                http::Event::Head(head) => {
                    *response_status = Some(head.status);
                    match head
                        .get("content-type")
                        .map(|value| {
                            value
                                .split(';')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_ascii_lowercase()
                        })
                        .as_deref()
                    {
                        // The account endpoint has been observed sending chunked
                        // SSE without Content-Type. The body still must pass all
                        // SSE/JSON checks and contain validated model completion.
                        None | Some("text/event-stream") => Ok(()),
                        Some("application/json") => Err(Error::ContentType(ContentKind::Json)),
                        Some("text/html") => Err(Error::ContentType(ContentKind::Html)),
                        Some("text/plain") => Err(Error::ContentType(ContentKind::Text)),
                        _ => Err(Error::ContentType(ContentKind::Other)),
                    }
                }
                http::Event::Data(data) => {
                    *stream_started = true;
                    model.push(data, &mut progress)
                }
                http::Event::End => Err(Error::MissingTerminal),
            };
            if let Err(error) = result {
                failure = Some(error);
            } else {
                completed = model.is_finished();
            }
            if failure.is_some() || completed {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if let Some(error) = failure {
            return self.fail(error);
        }
        if completed {
            // The semantic terminal event proves completion. Discard/close the
            // connection; trailing HTTP bytes are never reused for another request.
            self.completed = true;
            return Ok(());
        }
        if let Err(error) = result {
            return self.fail(Error::Http(error));
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        let _ = self.fail(Error::Cancelled);
    }
    fn fail(&mut self, error: Error) -> Result<(), Error> {
        self.error = Some(error);
        self.http.close();
        self.model.cancel();
        Err(error)
    }
    pub fn finish(mut self) -> Result<Response, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.completed {
            self.http
                .finish(|_| ControlFlow::Continue(()))
                .map_err(Error::Http)?;
        }
        self.model.finish()
    }
}
