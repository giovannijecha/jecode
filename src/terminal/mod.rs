//! Inline terminal presentation; network and conversation ownership stay separate.
mod account;
mod account_login;
mod action_demo;
mod action_view;
mod activity_view;
mod approval_view;
mod block;
mod command_view;
mod commands;
mod composer;
mod diagnostics;
mod editor;
mod editor_visual;
#[cfg(any(test, windows, target_os = "linux"))]
mod input;
mod markdown;
mod menu;
mod model;
mod navigation;
mod platform;
mod prompt_history;
mod reconcile;
#[cfg(test)]
mod reconciliation_tests;
mod recovery;
#[cfg(test)]
mod recovery_tests;
mod render;
mod resize;
mod schedule;
mod session_browser;
mod spinner;
mod style;
mod text;
mod tool_activity;
mod tool_demo;
mod tool_view;
mod view;

use std::{
    io::{self, IsTerminal, Write},
    time::Instant,
};

const START_SEQUENCE: &[u8] = b"\r\x1b[?25l\x1b[?2004h";
const NAVIGATION_SEQUENCE: &[u8] = b"\x1b[0m\x1b[?2004l\x1b[?25h";
const EXIT_SEQUENCE: &[u8] = b"\x1b[0m\x1b[?2004l\x1b[?25h\r\n";

#[derive(Clone, Debug, Eq, PartialEq)]
enum Key {
    Text(String),
    Paste(String),
    PasteRejected(&'static str),
    Enter,
    Newline,
    Escape,
    Interrupt,
    Quit,
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    RetrieveQueued,
    AbandonRecovered,
    Home,
    End,
    DraftStart,
    DraftEnd,
    Backspace,
    Delete,
    WordBackspace,
    WordDelete,
    PageUp,
    PageDown,
    HistoryPrevious,
    HistoryNext,
    Tab,
}

/// Run an explicitly local demo. No credentials, network, commands or persistence.
pub fn demo() -> io::Result<()> {
    run(None, None, None, None)
}

/// Authenticate without opening or creating a conversation.
pub fn login() -> io::Result<bool> {
    account_login::run()
}

/// Start an account conversation with saved access and canonical history.
pub fn account(model: crate::session::Model) -> io::Result<()> {
    account_in(model, None)
}

/// The selected workspace carries its file-access policy; effects require approval.
pub fn account_in(
    model: crate::session::Model,
    workspace: Option<crate::workspace::Workspace>,
) -> io::Result<()> {
    let directory = crate::session::scope::Directory::open(
        workspace
            .as_ref()
            .map_or(std::path::Path::new("."), crate::workspace::Workspace::path),
    )?;
    run(Some(model), Some(directory), workspace, None)
}

pub fn configured_account(
    model: Option<crate::session::Model>,
    effort: Option<Option<String>>,
    directory: crate::session::scope::Directory,
    workspace: Option<crate::workspace::Workspace>,
    access: Option<crate::workspace::Access>,
) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    let settings = crate::state::settings::Settings::user()?;
    let selection = resolve_selection(model, effort.as_ref(), settings.model)?;
    run(
        Some(selection),
        Some(directory),
        workspace.map(|w| w.with_access(access.unwrap_or(settings.file_access))),
        None,
    )
}

fn resolve_selection(
    model: Option<crate::session::Model>,
    effort: Option<&Option<String>>,
    saved: crate::session::Model,
) -> io::Result<crate::session::Model> {
    let selection = if let Some(model) = model {
        model.with_effort(effort.and_then(|value| value.as_deref()))
    } else if let Some(effort) = effort {
        saved.with_effort(effort.as_deref())
    } else {
        Some(saved)
    }
    .ok_or_else(|| io::Error::other("invalid model and effort selection"))?;
    Ok(selection)
}

pub fn resume(id: &str, directory: crate::session::scope::Directory) -> io::Result<()> {
    let start = navigation::resume(id, &directory)?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    run(
        start.selected,
        start.directory,
        start.workspace,
        start.saved,
    )
}

/// Browse saved sessions; selection returns an ID from the same displayed list.
pub fn sessions(
    select: bool,
    directory: &crate::session::scope::Directory,
) -> io::Result<Option<String>> {
    session_browser::show(select, directory)
}

fn run(
    selected: Option<crate::session::Model>,
    directory: Option<crate::session::scope::Directory>,
    workspace: Option<crate::workspace::Workspace>,
    saved: Option<crate::session::persistence::Saved>,
) -> io::Result<()> {
    let mut start = navigation::Start {
        selected,
        directory,
        workspace,
        saved,
        prepared: None,
    };
    while let Some(next) = run_once(start)? {
        start = next;
    }
    Ok(())
}

fn run_once(start: navigation::Start) -> io::Result<Option<navigation::Start>> {
    let navigation::Start {
        selected,
        directory,
        workspace,
        saved,
        prepared,
    } = start;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    let color = std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let mut terminal = platform::Terminal::open()?;
    let mut trace = diagnostics::Trace::open()?;
    // Guard exists before any escape write so partial startup also restores state.
    let mut screen = Screen {
        newline_on_drop: true,
    };
    let mut output = io::stdout().lock();
    output.write_all(START_SEQUENCE)?;
    output.flush()?;
    let mut model = selected.map_or_else(
        || model::Model::new(Instant::now()),
        |selected| {
            account::model(
                selected,
                directory
                    .as_ref()
                    .map(crate::session::scope::Directory::path),
            )
        },
    );
    let location =
        directory.map(|directory| navigation::Location::new(directory, workspace.as_ref()));
    if let Some(view) = &mut model.account {
        view.access = location.as_ref().unwrap().access;
        view.file_tools = workspace.is_some();
    }
    let configured_motion = if selected.is_some() {
        crate::state::settings::Settings::user()?.reduced_motion
    } else {
        false
    };
    model.tools.reduced_motion = std::env::var_os("JECODE_REDUCED_MOTION")
        .map_or(configured_motion, |value| !value.is_empty() && value != "0");
    // Start only after terminal/diagnostic initialization succeeded. Closing the
    // UI drops this owner, cancels its operation and joins its worker.
    let mut session = if let Some(prepared) = prepared {
        Some(prepared)
    } else {
        match saved {
            Some(saved) => Some(crate::session::Session::resume(
                saved,
                &location.as_ref().unwrap().directory,
                workspace,
            )?),
            None => selected
                .map(|selected| {
                    crate::session::Session::with_directory(
                        selected,
                        location.as_ref().unwrap().directory.path(),
                        workspace,
                    )
                })
                .transpose()?,
        }
    };
    if let Some(session) = &mut session {
        model.prompt_history.load(session.take_initial_prompts());
        account::attach_queue(&mut model, session);
    }
    let mut renderer = render::Renderer::default();
    let mut layout = view::Layout::default();
    let mut resize = resize::Resize::default();
    let mut paint = schedule::PaintSchedule::default();
    paint.request();
    let mut transcript_pending = true;
    let mut previous_size = (0, 0);
    loop {
        let size = terminal.size()?;
        model.editor.set_columns(size.0.saturating_sub(4).max(1));
        approval_view::displayed(&mut model, size, false);
        if size != previous_size {
            trace.changed();
            paint.request();
            transcript_pending = true;
        }
        let now = Instant::now();
        let region = resize.region(size, previous_size, now);
        if paint.ready(now)
            && let Some(region) = region
        {
            let geometry_preview = region == resize::Region::Composer;
            let region = if region == resize::Region::Transcript && !transcript_pending {
                resize::Region::Composer
            } else {
                region
            };
            let frame = match region {
                resize::Region::Transcript => layout.frame(&model, size.0, size.1),
                resize::Region::Composer => {
                    renderer.with_chrome(view::chrome(&model, size.0, size.1))
                }
            };
            if terminal.size()? != size {
                continue;
            }
            let rows = frame.len();
            if trace.active() {
                trace.record("before", size, rows, 0, &terminal.diagnostic_position()?)?;
            }
            let changed = renderer.draw(frame, size, color);
            if !changed.is_empty() {
                output.write_all(changed.as_bytes())?;
                output.flush()?;
            }
            approval_view::displayed(&mut model, size, region == resize::Region::Transcript);
            if trace.active() {
                trace.record(
                    "after",
                    size,
                    rows,
                    changed.len(),
                    &terminal.diagnostic_position()?,
                )?;
            }
            previous_size = size;
            if region == resize::Region::Transcript {
                transcript_pending = false;
            }
            paint.painted(Instant::now());
            if geometry_preview {
                // The composer-only paint must not consume pending source work.
                paint.request();
            }
        }
        for key in terminal.poll()? {
            if let Some(session) = &mut session {
                account::input(&mut model, key, session);
            } else {
                model.input(key, Instant::now());
            }
            paint.request();
            transcript_pending = true;
            if model.quit {
                return Ok(None);
            }
            if let Some(request) = model.navigation.take() {
                match location.as_ref().unwrap().resolve(request) {
                    Ok(mut next) => {
                        if next.prepare().is_err() {
                            if let Some(view) = &mut model.account {
                                view.local_notice = "Cannot open that conversation · check its directory or another owner · current session and draft kept".into();
                            }
                            continue;
                        }
                        let frame = renderer.with_chrome(Vec::new());
                        output
                            .write_all(renderer.draw(frame, terminal.size()?, color).as_bytes())?;
                        output.flush()?;
                        screen.newline_on_drop = false;
                        // The current worker and lease are joined/released before the next run.
                        drop(session);
                        return Ok(Some(next));
                    }
                    Err(_) => {
                        if let Some(view) = &mut model.account {
                            view.local_notice = "Cannot open that conversation · check its folder or another running owner · current session kept".into();
                            view.local_failed = false;
                        }
                    }
                }
            }
        }
        if let Some(session) = &mut session {
            for _ in 0..64 {
                let Some(event) = session.poll() else { break };
                account::event(&mut model, event);
                paint.request();
                transcript_pending = true;
            }
        }
        if model.tick(Instant::now()) {
            paint.request();
            // Account ticks only change transient animation and elapsed status.
            // Demo ticks may also append scripted transcript output.
            transcript_pending |= model.account.is_none();
        }
    }
}

struct Screen {
    newline_on_drop: bool,
}
impl Drop for Screen {
    fn drop(&mut self) {
        let mut out = io::stdout().lock();
        let sequence = if self.newline_on_drop {
            EXIT_SEQUENCE
        } else {
            NAVIGATION_SEQUENCE
        };
        let _ = out.write_all(sequence);
        let _ = out.flush();
    }
}

#[cfg(test)]
mod action_tests;
#[cfg(test)]
mod composer_tests;
#[cfg(test)]
mod consistency_tests;
#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
mod editor_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(all(test, windows))]
mod menu_native_tests;
#[cfg(test)]
mod menu_tests;
#[cfg(test)]
mod performance_tests;
#[cfg(test)]
mod reflow_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod stream_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tool_activity_tests;
#[cfg(test)]
#[path = "../../tests/support/vt.rs"]
mod vt;
