use super::*;

struct Clocked<'a> {
    origin: Instant,
    clock: &'a std::cell::Cell<Instant>,
    reads: VecDeque<(u64, Vec<u8>)>,
    cancel_on_read: Option<&'a AtomicBool>,
    calls: usize,
}
impl ResponseChannel for Clocked<'_> {
    fn write(
        &mut self,
        bytes: &[u8],
        _: &Budget<'_>,
        progress: &mut ApplicationWrite,
    ) -> Result<(), NetworkError> {
        progress.accepted_wire_bytes = bytes.len();
        Ok(())
    }
    fn read(&mut self, _: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
        self.calls += 1;
        let Some((seconds, bytes)) = self.reads.pop_front() else {
            return Ok(None);
        };
        self.clock.set(self.origin + Duration::from_secs(seconds));
        if let Some(cancelled) = self.cancel_on_read {
            cancelled.store(true, Ordering::Release);
        }
        Ok(Some(Plaintext {
            kind: ContentType::Application,
            bytes,
        }))
    }
    fn close(&mut self, _: &Budget<'_>) -> Result<(), NetworkError> {
        Ok(())
    }
}
fn clocked_exchange<'a>(
    channel: &mut Clocked<'a>,
    budget: &Budget<'_>,
    trace: &mut recovery::Trace,
    delivery: &mut Delivery,
    clock: &'a std::cell::Cell<Instant>,
) -> Result<Response, Error> {
    let mut write = ApplicationWrite::default();
    exchange::exchange_with_clock(
        channel,
        b"fixture request",
        budget,
        exchange::RequestWindows {
            write: budget,
            idle: Duration::from_secs(120),
        },
        ExchangeProgress {
            delivery,
            write: &mut write,
            trace,
        },
        |_| ControlFlow::Continue(()),
        || clock.get(),
    )
}
fn data_event(kind: &str) -> String {
    format!("data: {{\"type\":\"{kind}\",\"response\":{{\"id\":\"r1\"}}}}\n\n")
}
fn terminal_event() -> String {
    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[]}}\n\n".into()
}
fn clocked_channel<'a>(
    origin: Instant,
    clock: &'a std::cell::Cell<Instant>,
    pieces: Vec<(u64, Vec<u8>)>,
) -> Clocked<'a> {
    Clocked {
        origin,
        clock,
        reads: pieces.into(),
        cancel_on_read: None,
        calls: 0,
    }
}

#[test]
fn progressing_non_text_sse_completes_past_old_compaction_and_generation_limits() {
    let origin = Instant::now();
    let clock = std::cell::Cell::new(origin);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        deadline: None,
        cancelled: &cancelled,
    };
    let first = data_event("response.created");
    let reasoning =
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"reasoning\"}\n\n";
    let mut pieces: Vec<(u64, String)> = vec![(1, first.clone())];
    for n in 1..=6 {
        pieces.push((
            n * 100,
            if n % 2 == 0 {
                reasoning.into()
            } else {
                data_event("response.in_progress")
            },
        ));
    }
    pieces.push((700, terminal_event()));
    let body: String = pieces.iter().map(|(_, part)| part.as_str()).collect();
    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
    pieces[0].1.insert_str(0, &header);
    let mut channel = clocked_channel(
        origin,
        &clock,
        pieces
            .into_iter()
            .map(|(at, part)| (at, part.into_bytes()))
            .collect(),
    );
    let mut trace = recovery::Trace::default();
    let mut delivery = Delivery::NotSubmitted;
    let response =
        clocked_exchange(&mut channel, &budget, &mut trace, &mut delivery, &clock).unwrap();
    assert_eq!(
        response.status,
        crate::providers::openai_account::Status::Completed
    );
    assert_eq!(delivery, Delivery::Completed);
    assert_eq!(trace.stream_events, 8);
    assert_eq!(trace.request_elapsed_ms, 700_000);
    assert_eq!(channel.calls, 8);
}

#[test]
fn headers_comments_and_fragmented_data_cannot_renew_first_response_wait() {
    let origin = Instant::now();
    let clock = std::cell::Cell::new(origin);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        deadline: None,
        cancelled: &cancelled,
    };
    let first = data_event("response.created");
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        first.len() + terminal_event().len()
    );
    let mut channel = clocked_channel(
        origin,
        &clock,
        vec![
            (10, format!("{header}: keepalive\n\ndata: {{").into_bytes()),
            (121, first.into_bytes()),
        ],
    );
    let mut trace = recovery::Trace::default();
    let mut delivery = Delivery::NotSubmitted;
    let error =
        clocked_exchange(&mut channel, &budget, &mut trace, &mut delivery, &clock).unwrap_err();
    assert!(matches!(
        error,
        Error::Transport {
            stage: RequestStage::ResponseRead,
            error: NetworkError::Timeout,
            ..
        }
    ));
    assert_eq!(trace.termination, Some(Termination::FirstResponseTimeout));
    assert_eq!(trace.stream_events, 0);
    assert_eq!(trace.since_progress_ms, Some(121_000));
    assert_eq!(channel.calls, 2);
}

#[test]
fn keepalive_does_not_renew_idle_and_explicit_budget_precedes_idle() {
    let origin = Instant::now();
    let clock = std::cell::Cell::new(origin);
    let cancelled = AtomicBool::new(false);
    let first = data_event("response.created");
    let body = format!("{first}: keepalive\n\n{}", terminal_event());
    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
    let pieces = vec![
        (1, format!("{header}{first}").into_bytes()),
        (100, b": keepalive\n\n".to_vec()),
        (122, terminal_event().into_bytes()),
    ];
    let mut channel = clocked_channel(origin, &clock, pieces.clone());
    let budget = Budget {
        deadline: None,
        cancelled: &cancelled,
    };
    let mut trace = recovery::Trace::default();
    let mut delivery = Delivery::NotSubmitted;
    assert!(clocked_exchange(&mut channel, &budget, &mut trace, &mut delivery, &clock).is_err());
    assert_eq!(trace.termination, Some(Termination::IdleTimeout));
    assert_eq!(trace.stream_events, 1);
    clock.set(origin);
    let mut channel = clocked_channel(origin, &clock, pieces);
    let budget = Budget {
        deadline: Some(origin + Duration::from_secs(90)),
        cancelled: &cancelled,
    };
    let mut trace = recovery::Trace::default();
    let mut delivery = Delivery::NotSubmitted;
    assert!(clocked_exchange(&mut channel, &budget, &mut trace, &mut delivery, &clock).is_err());
    assert_eq!(trace.termination, Some(Termination::TotalBudget));
}

#[test]
fn cancellation_after_fragmented_read_is_prompt_and_not_retried() {
    let origin = Instant::now();
    let clock = std::cell::Cell::new(origin);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        deadline: None,
        cancelled: &cancelled,
    };
    let mut channel = clocked_channel(origin, &clock, vec![(1, b"HTTP/1.1 200 OK\r\n".to_vec())]);
    channel.cancel_on_read = Some(&cancelled);
    let mut trace = recovery::Trace::default();
    let mut delivery = Delivery::NotSubmitted;
    let error =
        clocked_exchange(&mut channel, &budget, &mut trace, &mut delivery, &clock).unwrap_err();
    assert!(matches!(
        error,
        Error::Transport {
            error: NetworkError::Cancelled,
            delivery: Delivery::PossiblySubmitted,
            ..
        }
    ));
    assert_eq!(trace.termination, Some(Termination::Cancelled));
    assert_eq!(channel.calls, 1);
}

#[test]
fn slow_local_progress_callback_does_not_renew_provider_idle_time() {
    let origin = Instant::now();
    let clock = std::cell::Cell::new(origin);
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        deadline: None,
        cancelled: &cancelled,
    };
    let reasoning =
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"thinking\"}\n\n";
    let body = format!("{reasoning}{}", terminal_event());
    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
    let mut channel = clocked_channel(
        origin,
        &clock,
        vec![
            (1, format!("{header}{reasoning}").into_bytes()),
            (201, terminal_event().into_bytes()),
        ],
    );
    let mut trace = recovery::Trace::default();
    let mut delivery = Delivery::NotSubmitted;
    let mut write = ApplicationWrite::default();
    let error = exchange::exchange_with_clock(
        &mut channel,
        b"fixture request",
        &budget,
        exchange::RequestWindows {
            write: &budget,
            idle: Duration::from_secs(120),
        },
        ExchangeProgress {
            delivery: &mut delivery,
            write: &mut write,
            trace: &mut trace,
        },
        |progress| {
            if matches!(progress, Progress::Reasoning(_)) {
                clock.set(origin + Duration::from_secs(200));
            }
            ControlFlow::Continue(())
        },
        || clock.get(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error::Transport {
            error: NetworkError::Timeout,
            ..
        }
    ));
    assert_eq!(trace.termination, Some(Termination::IdleTimeout));
    assert_eq!(trace.since_progress_ms, Some(199_000));
    assert_eq!(channel.calls, 1);
}
