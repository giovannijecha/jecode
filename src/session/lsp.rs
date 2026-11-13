use crate::agent::Agent;
use std::{io::Write, path::Path};

pub(super) fn print(agent: &mut Agent, home: &Path, output: &mut impl Write) -> Result<(), String> {
    agent.configure_lsp(home)?;
    let profiles = crate::lsp::Store::new(home.to_path_buf()).load()?;
    let observed = agent
        .lsp_runtime()
        .status(&agent.archive().directory, &profiles, None)
        .servers;
    let summary = format!("LSP · {} configured", profiles.len());
    let mut lines = vec![summary.clone(), "Opening this view starts nothing.".into()];
    for profile in profiles {
        let instances = observed.iter().filter(|status| status.server == profile.id);
        for status in instances {
            lines.push(format!(
                "{} ({}) · {}",
                profile.name,
                profile.id,
                status.state.replace('_', " ")
            ));
            if let Some(root) = &status.root {
                lines.push(format!("  Root: {}", root.display()));
            }
            if !status.detail.is_empty() {
                lines.push(format!("  {}", status.detail));
            }
        }
    }
    lines.push(
        "Manage servers with jecode lsp, in Settings > LSP in the TUI, or in ~/.jecode/lsp.json."
            .into(),
    );
    agent.record_local("/lsp", &summary);
    writeln!(output, "{}", agent.redactor().text(&lines.join("\n")))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::openrouter::OpenRouter,
        config::{Settings, Store},
        session::{SessionConfig, chat},
        test_support::{Directory, HttpFixture},
        tools::Tools,
    };
    use std::io::Cursor;

    #[test]
    fn plain_lsp_inspection_is_read_only_redacted_and_uses_no_provider() {
        let server = crate::lsp::fixture::Fixture::new();
        let home = Directory::new();
        let mut profile = server.profile();
        profile.name = "Server isolated-lsp-secret".into();
        profile
            .environment
            .insert("TEST_SECRET".into(), "isolated-lsp-secret".into());
        crate::lsp::Store::new(home.path().to_path_buf())
            .save(profile)
            .unwrap();
        let provider = HttpFixture::new(vec![]);
        let mut agent = Agent::new(
            OpenRouter::fixture(provider.endpoint.clone()),
            Tools::new(server.workspace()).unwrap(),
        );
        let mut config = SessionConfig {
            store: Store::new(home.path().to_path_buf()),
            settings: Settings::new("isolated-fixture-key".into(), "fixture/model".into()).unwrap(),
        };
        let mut output = Vec::new();
        let mut status = Vec::new();
        chat(
            &mut agent,
            &mut config,
            &mut Cursor::new("/lsp\n/exit\n"),
            &mut output,
            &mut status,
            Vec::new(),
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(
            text.contains("LSP · 1 configured") && text.contains("not started"),
            "{text}"
        );
        assert!(!text.contains("isolated-lsp-secret"));
        assert!(status.is_empty(), "{}", String::from_utf8_lossy(&status));
        assert!(!server.marker().exists());
        assert!(agent.lsp_runtime().statuses().is_empty());
        assert!(provider.finish().is_empty());
    }
}
