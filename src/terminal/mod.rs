//! Inline terminal presentation; network and conversation ownership stay separate.
mod account;
mod action_demo;
mod action_view;
mod approval_view;
mod block;
mod command_view;
mod commands;
mod diagnostics;
#[cfg(any(test, windows, target_os = "linux"))]
mod input;
mod markdown;
mod menu;
mod model;
mod navigation;
mod platform;
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

#[derive(Debug, Eq, PartialEq)]
enum Key {
    Text(String),
    Enter,
    Escape,
    Interrupt,
    Quit,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    PageUp,
    PageDown,
    Tab,
}

/// Run an explicitly local demo. No credentials, network, commands or persistence.
pub fn demo() -> io::Result<()> {
    run(None, None, None)
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
    run(Some(model), workspace, None)
}

pub fn configured_account(
    model: Option<crate::session::Model>,
    workspace: Option<crate::workspace::Workspace>,
    access: Option<crate::workspace::Access>,
) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    let settings = crate::state::settings::Settings::user()?;
    account_in(
        model.unwrap_or(settings.model),
        workspace.map(|w| w.with_access(access.unwrap_or(settings.file_access))),
    )
}

pub fn resume(id: &str) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    let start = navigation::resume(id)?;
    run(start.selected, start.workspace, start.saved)
}

/// Browse saved sessions; selection returns an ID from the same displayed list.
pub fn sessions(select: bool) -> io::Result<Option<String>> {
    session_browser::show(select)
}

fn run(
    selected: Option<crate::session::Model>,
    workspace: Option<crate::workspace::Workspace>,
    saved: Option<crate::session::persistence::Saved>,
) -> io::Result<()> {
    let mut start = navigation::Start {
        selected,
        workspace,
        saved,
    };
    while let Some(next) = run_once(start)? {
        start = next;
    }
    Ok(())
}

fn run_once(start: navigation::Start) -> io::Result<Option<navigation::Start>> {
    let navigation::Start {
        selected,
        workspace,
        saved,
    } = start;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    let color = std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
    let mut terminal = platform::Terminal::open()?;
    let mut trace = diagnostics::Trace::open()?;
    // Guard exists before any escape write so partial startup also restores state.
    let _screen = Screen;
    let mut output = io::stdout().lock();
    output.write_all(b"\r\n\x1b[?25l\x1b[?2004h")?;
    output.flush()?;
    let mut model = selected.map_or_else(
        || model::Model::new(Instant::now()),
        |selected| {
            account::model(
                selected,
                workspace.as_ref().map(crate::workspace::Workspace::path),
            )
        },
    );
    let location = navigation::Location::from_workspace(workspace.as_ref());
    if workspace.is_some() {
        model.blocks.push(model::Block {
            speaker: "Status",
            text: match location.access {
                crate::workspace::Access::Local => {
                    "Access · local paths, including outside this directory"
                }
                crate::workspace::Access::Workspace => "Access · paths within this workspace",
            }
            .into(),
        });
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
    let mut session = match saved {
        Some(saved) => Some(crate::session::Session::resume(saved, workspace)?),
        None => selected
            .map(|selected| crate::session::Session::with_workspace(selected, workspace))
            .transpose()?,
    };
    let mut renderer = render::Renderer::default();
    let mut layout = view::Layout::default();
    let mut resize = resize::Resize::default();
    let mut paint = schedule::PaintSchedule::default();
    paint.request();
    let mut previous_size = (0, 0);
    loop {
        let size = terminal.size()?;
        approval_view::displayed(&mut model, size, false);
        if size != previous_size {
            trace.changed();
            paint.request();
        }
        let now = Instant::now();
        let region = resize.region(size, previous_size, now);
        if paint.ready(now)
            && let Some(region) = region
        {
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
            paint.painted(Instant::now());
            if region == resize::Region::Composer {
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
            if model.quit {
                return Ok(None);
            }
            if let Some(request) = model.navigation.take() {
                match location.resolve(request) {
                    Ok(next) => {
                        let frame = renderer.with_chrome(Vec::new());
                        output
                            .write_all(renderer.draw(frame, terminal.size()?, color).as_bytes())?;
                        output.flush()?;
                        // The current worker and lease are joined/released before the next run.
                        drop(session);
                        return Ok(Some(next));
                    }
                    Err(_) => {
                        if let Some(view) = &mut model.account {
                            view.notice = "Cannot open that conversation · check its folder or another running owner · current session kept".into();
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
            }
        }
        if model.tick(Instant::now()) {
            paint.request();
        }
    }
}

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(b"\x1b[0m\x1b[?2004l\x1b[?25h\r\n");
        let _ = out.flush();
    }
}

#[cfg(test)]
mod action_tests;
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
