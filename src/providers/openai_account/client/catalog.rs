use super::{Client, Delivery, Error, RequestStage, ResponseChannel, unix_seconds};
use crate::{
    http,
    providers::openai_account::catalog::Catalog,
    tls::{ApplicationWrite, Budget, Connection, NetworkError, trust::TrustStore},
};
use std::{
    ops::ControlFlow,
    time::{Duration, Instant},
};

impl Client {
    /// Fetch at most once per five minutes for this signed-in client. Credential
    /// checks bracket network I/O; no credential lease spans the response.
    pub fn catalog(&mut self, budget: &Budget<'_>) -> Result<Catalog, Error> {
        self.catalog_with(budget, |trust, connect| {
            Connection::connect("chatgpt.com", trust, connect)
        })
    }

    fn catalog_with<C: ResponseChannel>(
        &mut self,
        budget: &Budget<'_>,
        connect_channel: impl FnOnce(&TrustStore, &Budget<'_>) -> Result<C, NetworkError>,
    ) -> Result<Catalog, Error> {
        budget.check()?;
        self.ensure_access(budget)?;
        if let Some((at, catalog)) = &self.catalog
            && at.elapsed() < Duration::from_secs(300)
        {
            return Ok(catalog.clone());
        }
        let short = Budget {
            cancelled: budget.cancelled,
            deadline: budget.deadline.min(Instant::now() + Duration::from_secs(5)),
        };
        let mut connection =
            connect_channel(&self.trust, &short).map_err(|error| Error::Transport {
                stage: RequestStage::Connect,
                error,
                delivery: Delivery::NotSubmitted,
                accepted_wire_bytes: 0,
            })?;
        self.ensure_access(&short)?;
        if unix_seconds()? >= self.tokens.expires_at().saturating_sub(30) {
            return Err(Error::Expired);
        }
        let token = self.tokens.access_token();
        if token.is_empty() || token.len() > 8192 || !token.bytes().all(|b| (33..=126).contains(&b))
        {
            return Err(Error::Protocol(
                crate::providers::openai_account::Error::InvalidRequest,
            ));
        }
        let bytes = http::get(
            "chatgpt.com",
            crate::providers::openai_account::catalog::PATH,
            &[
                ("Authorization", &format!("Bearer {token}")),
                ("ChatGPT-Account-Id", self.tokens.account_id()),
                ("Accept", "application/json"),
            ],
            32768,
        )?;
        let result = exchange(&mut connection, &bytes, &short)?;
        self.ensure_access(&short)?;
        self.catalog = Some((Instant::now(), result.clone()));
        Ok(result)
    }
}

fn exchange(
    channel: &mut impl ResponseChannel,
    request: &[u8],
    budget: &Budget<'_>,
) -> Result<Catalog, Error> {
    let mut write = ApplicationWrite::default();
    channel
        .write(request, budget, &mut write)
        .map_err(|error| Error::Transport {
            stage: RequestStage::RequestWrite,
            error,
            delivery: if write.accepted_wire_bytes == 0 {
                Delivery::NotSubmitted
            } else {
                Delivery::PossiblySubmitted
            },
            accepted_wire_bytes: write.accepted_wire_bytes,
        })?;
    let mut decoder = http::Decoder::new(http::Limits {
        header_bytes: 16 * 1024,
        body_bytes: crate::providers::openai_account::catalog::MAX_BYTES,
        framing_bytes: 1024 * 1024,
    });
    let mut body = Vec::new();
    let mut response_error = None;
    while !decoder.is_complete() {
        let Some(bytes) = channel.read(budget).map_err(|error| Error::Transport {
            stage: RequestStage::ResponseRead,
            error,
            delivery: Delivery::PossiblySubmitted,
            accepted_wire_bytes: write.accepted_wire_bytes,
        })?
        else {
            break;
        };
        let result = decoder.push(&bytes.bytes, |event| {
            match event {
                http::Event::Head(head) if head.status != 200 => {
                    response_error = Some(Error::Status(head.status))
                }
                http::Event::Head(head) => {
                    if head.get("content-type").is_some_and(|value| {
                        !value
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .eq_ignore_ascii_case("application/json")
                    }) {
                        response_error = Some(Error::Content);
                    }
                }
                http::Event::Data(data) => body.extend_from_slice(data),
                http::Event::End => {}
            }
            if response_error.is_some() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        if let Some(error) = response_error {
            return Err(error);
        }
        result.map_err(Error::Http)?;
    }
    decoder.finish(|_| ControlFlow::Continue(()))?;
    let _ = channel.close(&Budget {
        cancelled: budget.cancelled,
        deadline: budget
            .deadline
            .min(Instant::now() + Duration::from_millis(100)),
    });
    Catalog::parse(&body).map_err(Error::Catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        providers::openai_account::auth,
        tls::{ContentType, Plaintext},
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    const TOKEN: &str = "e30.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC10ZXN0In19.c2ln";
    fn client() -> Client {
        let saved = format!(
            r#"{{"version":1,"state":"ready","provider":"openai-account","access_token":"{TOKEN}","refresh_token":"synthetic","account_id":"account-test","expires_at":9999999999,"generation":"a1"}}"#
        );
        Client {
            trust: TrustStore::native().unwrap(),
            tokens: auth::Tokens::from_saved_json(&saved).unwrap().unwrap(),
            store: None,
            catalog: None,
        }
    }
    fn response(status: u16, content_type: &str, body: &str) -> Vec<u8> {
        format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }
    struct Channel {
        response: Option<Vec<u8>>,
        sent: Arc<Mutex<Vec<u8>>>,
        cancel_on_read: bool,
    }
    impl ResponseChannel for Channel {
        fn write(
            &mut self,
            bytes: &[u8],
            _: &Budget<'_>,
            progress: &mut ApplicationWrite,
        ) -> Result<(), NetworkError> {
            self.sent.lock().unwrap().extend_from_slice(bytes);
            progress.accepted_wire_bytes += bytes.len();
            Ok(())
        }
        fn read(&mut self, budget: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
            if self.cancel_on_read {
                budget.cancelled.store(true, Ordering::Release);
            }
            budget.check()?;
            Ok(self.response.take().map(|bytes| Plaintext {
                kind: ContentType::Application,
                bytes,
            }))
        }
        fn close(&mut self, _: &Budget<'_>) -> Result<(), NetworkError> {
            Ok(())
        }
    }
    #[test]
    fn exact_account_get_is_bounded_cached_and_contains_no_token_in_metadata() {
        let mut client = client();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let body = r#"{"models":[{"slug":"model-one","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]}]}"#;
        let catalog = client
            .catalog_with(&budget, |_, _| {
                Ok(Channel {
                    response: Some(response(200, "application/json", body)),
                    sent: sent.clone(),
                    cancel_on_read: false,
                })
            })
            .unwrap();
        assert_eq!(catalog.entries[0].id, "model-one");
        let wire = String::from_utf8(sent.lock().unwrap().clone()).unwrap();
        assert!(wire.starts_with(&format!(
            "GET {} HTTP/1.1\r\nHost: chatgpt.com\r\n",
            crate::providers::openai_account::catalog::PATH
        )));
        assert!(wire.contains(&format!("Authorization: Bearer {TOKEN}\r\n")));
        assert!(wire.contains("ChatGPT-Account-Id: account-test\r\n"));
        assert!(!format!("{catalog:?}").contains(TOKEN));
        assert_eq!(
            client
                .catalog_with::<Channel>(&budget, |_, _| panic!("fresh catalog must be reused"))
                .unwrap(),
            catalog
        );
    }
    #[test]
    fn failure_and_cancellation_do_not_cache_a_catalog() {
        for (wire, expected) in [
            (response(403, "application/json", "{}"), Error::Status(403)),
            (response(200, "text/html", "secret"), Error::Content),
            (
                response(200, "application/json", "{}"),
                Error::Catalog(crate::providers::openai_account::catalog::Error::Malformed),
            ),
        ] {
            let mut client = client();
            let cancelled = AtomicBool::new(false);
            let budget = Budget {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(1),
            };
            let result = client.catalog_with(&budget, |_, _| {
                Ok(Channel {
                    response: Some(wire),
                    sent: Arc::new(Mutex::new(Vec::new())),
                    cancel_on_read: false,
                })
            });
            assert_eq!(result, Err(expected));
            assert!(client.catalog.is_none());
            assert!(!expected.to_string().contains("secret"));
        }
        let mut client = client();
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let result = client.catalog_with(&budget, |_, _| {
            Ok(Channel {
                response: None,
                sent: Arc::new(Mutex::new(Vec::new())),
                cancel_on_read: true,
            })
        });
        assert!(matches!(
            result,
            Err(Error::Transport {
                stage: RequestStage::ResponseRead,
                error: NetworkError::Cancelled,
                delivery: Delivery::PossiblySubmitted,
                accepted_wire_bytes: 1..,
            })
        ));
        assert!(client.catalog.is_none());
    }

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn logout_while_catalog_is_in_flight_cannot_revalidate_old_metadata() {
        use std::sync::mpsc;
        let fixture = crate::state::tests::Fixture::new();
        let Some(store) = fixture.store() else { return };
        let mut client = client();
        store
            .replace("credentials.json", &client.tokens.saved_json().unwrap())
            .unwrap();
        client.store = Some(store.clone());
        struct Blocked {
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
            response: Option<Vec<u8>>,
        }
        impl ResponseChannel for Blocked {
            fn write(
                &mut self,
                bytes: &[u8],
                _: &Budget<'_>,
                progress: &mut ApplicationWrite,
            ) -> Result<(), NetworkError> {
                progress.accepted_wire_bytes += bytes.len();
                Ok(())
            }
            fn read(&mut self, budget: &Budget<'_>) -> Result<Option<Plaintext>, NetworkError> {
                self.entered.send(()).unwrap();
                self.release
                    .recv_timeout(Duration::from_secs(3))
                    .map_err(|_| NetworkError::Timeout)?;
                budget.check()?;
                Ok(self.response.take().map(|bytes| Plaintext {
                    kind: ContentType::Application,
                    bytes,
                }))
            }
            fn close(&mut self, _: &Budget<'_>) -> Result<(), NetworkError> {
                Ok(())
            }
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let cancelled = AtomicBool::new(false);
            let budget = Budget {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(4),
            };
            let body = r#"{"models":[{"slug":"model-one","visibility":"list"}]}"#;
            let result = client.catalog_with(&budget, |_, _| {
                Ok(Blocked {
                    entered: entered_tx,
                    release: release_rx,
                    response: Some(response(200, "application/json", body)),
                })
            });
            (result, client.catalog)
        });
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let cancelled = AtomicBool::new(false);
        let budget = Budget {
            cancelled: &cancelled,
            deadline: Instant::now() + Duration::from_secs(2),
        };
        Client::logout_in(store.clone(), &budget).unwrap();
        release_tx.send(()).unwrap();
        let (result, cache) = worker.join().unwrap();
        assert_eq!(result, Err(Error::AccountChanged));
        assert!(cache.is_none());
        assert!(
            store
                .read("credentials.json", 32768)
                .unwrap()
                .unwrap()
                .contains("signed_out")
        );
    }
}
