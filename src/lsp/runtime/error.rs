pub(super) enum QueryFailure {
    Controlled(&'static str, String),
    Transport(String),
}

impl QueryFailure {
    pub(super) fn classify_local(error: String) -> Self {
        // The transport refuses an oversized message before writing anything, so the server stays healthy.
        if error.starts_with("LSP message exceeds") {
            Self::Controlled(
                "unavailable",
                format!("LSP document too large to synchronize: {error}"),
            )
        } else if error.starts_with("Cannot read")
            || error.starts_with("LSP document must")
            || error.contains("No such file")
        {
            Self::Controlled("unavailable", error)
        } else {
            Self::Transport(error)
        }
    }
    pub(super) fn classify_protocol(error: String) -> Self {
        if error.starts_with("LSP server error") {
            Self::Controlled("server_error", error)
        } else if error.contains("settings changed") {
            Self::Controlled("settings_changed", error)
        } else if error.contains("cancelled") {
            Self::Controlled("cancelled", error)
        } else if error.contains("timed out") {
            Self::Controlled("timed_out", error)
        } else {
            Self::Transport(error)
        }
    }
}
