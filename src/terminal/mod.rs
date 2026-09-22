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
mod model;
mod platform;
mod render;
mod resize;
mod schedule;
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

/// Only this selected root is available to reads and individually approved changes.
pub fn account_in(
    model: crate::session::Model,
    workspace: Option<crate::workspace::Workspace>,
) -> io::Result<()> {
    run(Some(model), workspace, None)
}

pub fn configured_account(
    model: Option<crate::session::Model>,
    workspace: Option<crate::workspace::Workspace>,
) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    let settings = crate::state::settings::Settings::user()?;
    account_in(model.unwrap_or(settings.model), workspace)
}

pub fn resume(id: &str) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("Jecode needs an interactive terminal"));
    }
    let saved = crate::session::persistence::resume(id)?;
    let workspace = saved
        .workspace
        .as_ref()
        .map(|path| {
            crate::workspace::Workspace::open(path)
                .map_err(|_| io::Error::other("saved workspace is unavailable"))
        })
        .transpose()?;
    run(Some(saved.model), workspace, Some(saved))
}

fn run(
    selected: Option<crate::session::Model>,
    workspace: Option<crate::workspace::Workspace>,
    saved: Option<crate::session::persistence::Saved>,
) -> io::Result<()> {
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
                return Ok(());
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
