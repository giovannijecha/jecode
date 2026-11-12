# Commands and keys

The complete list of slash commands and keyboard controls in the TUI. `/help`
(or F1) shows the command list and common shortcuts inside Jecode. Plain-chat
commands are listed in
[Command line](../usage/CLI.md#plain-chat).

## Slash commands

Type `/` to see the commands grouped as below, or press Ctrl+G for the same
catalogue, groups and order in a **Commands** sheet (see [Keys](#keys)).
**During a turn** says what happens if you run a command while the agent is
working: *now* runs it immediately, *opens* shows the menu at once and applies
conversation changes from the next turn, *waits* queues it until the turn
completes and any open menu or information panel closes. If the turn stops or
fails, a waiting command returns to the composer as a draft, or stays `paused`,
and does not run. The Ctrl+G sheet never queues: commands that wait stay listed in dim text marked
`after this turn` and cannot be selected.

### Conversation

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/new` | Start a new conversation with your saved defaults. Alias: `/clear`. | Waits |
| `/resume [ID]` | Open a saved conversation from this folder, or delete one. See [Saved conversations](../usage/SESSIONS.md). | Waits |
| `/export` | Save the conversation as a JSON file in the working directory. | Waits |
| `/copy` | Copy the latest answer or one of its code blocks or quotes. See [Copy and export](../usage/COPY_AND_EXPORT.md). | Now |
| `/agents` | Inspect collaborators. See [Collaborating agents](../usage/COLLABORATION.md#watch-in-the-tui). | Now |
| `/ask` | Return to questions the agent is waiting on. | Now |
| `/todo` | Show the conversation's plan. See [Questions and plans](../usage/QUESTIONS_AND_PLANS.md#plans). | Now |
| `/exit` | Quit after pending work completes. Alias: `/quit`. | Waits |

### Model

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/model [ID]` | Choose API keys or Credentials, then provider, model and effort for this conversation. An ID switches the current provider's model directly. | Opens |
| `/effort [NAME]` | Choose the reasoning effort for this conversation. | Opens |
| `/routing` | Choose an OpenRouter endpoint for this conversation. See [Configuration](CONFIGURATION.md#openrouter-endpoint-routing). | Opens |
| `/permissions [LEVEL]` | Choose Read, Workspace or Full for this conversation, or review remembered approvals. See [Permissions](../usage/PERMISSIONS.md). | Now |

### Workspace

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/init` | Have the model create or update the project's `AGENTS.md`. See [Project instructions](../usage/PROJECT_INSTRUCTIONS.md). | Waits |
| `/skills` | List your personal skills. See [Skills](../usage/SKILLS.md). | Opens |
| `/tmp [clean]` | Show, or clear, the conversation's temporary files. See [Temporary files](../usage/TEMPORARY.md). | Waits |
| `/process [ID]` | Inspect, stop, background or type into running and recent processes; `/process stop ID` stops one. See [Terminal interface](../usage/INTERFACE.md#processes). | Now |

### Jecode

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/settings` | Change saved defaults and connect providers. Alias: `/setup`. See [Configuration](CONFIGURATION.md). | Opens |
| `/mcp` | Inspect globally configured MCP connections and their tools. Configure them in Settings → MCP. | Now |
| `/lsp` | Inspect configured language servers and this conversation's observed instances without starting one. Configure in Settings → LSP. | Now |
| `/help` | Show commands and keys, including Ctrl+G. | Opens |

An unknown command reports an error, with a suggestion when the name is close to
a real one.

## Keys

### Composer

| Key | Action |
| --- | --- |
| Enter | Send the draft, or queue it while the agent works. |
| Ctrl+J | Insert a newline. Shift+Enter and Alt+Enter also work when the terminal reports them; on Linux and macOS, Jecode asks for that through the keyboard protocol terminals such as kitty, Ghostty, WezTerm and iTerm2 support. |
| Left, Right | Move by character. |
| Ctrl+Left, Ctrl+Right | Move by word. |
| Up, Down | Move between rows of the draft. In an empty composer, Up takes back the pending message. |
| Home, End, or Ctrl+A, Ctrl+E | Go to the start or end of the line. |
| Ctrl+Home, Ctrl+End | Go to the start or end of the draft. |
| Backspace, Delete | Delete a character, or a whole attachment. |
| Ctrl+Backspace, Ctrl+Delete, Ctrl+W | Delete a word, when the terminal reports the shortcut. |
| Ctrl+U | Delete to the start of the line. |
| Ctrl+P, Ctrl+N | Browse your last 50 sent messages; Esc returns to the draft. |
| Alt+V, or Ctrl+V on Linux and macOS | Attach an image from the clipboard. On Windows, Ctrl+V stays your terminal's text paste. |
| `/` | Open the command list; Up/Down select, Tab completes, Enter accepts the selected full command and runs or queues it. Use it to type arguments; commands with arguments keep their typed text. |
| `$` | Typed as a word of its own, open the **Skills** sheet; Enter or Tab writes `$name` into the draft. See [Reference skills and files](../usage/INTERFACE.md#reference-skills-and-files). |
| `@` | Typed as a word of its own, open the **Files** sheet at the working directory; Enter or Tab opens a folder or writes `@path` for a file. |
| Tab | With the caret on a `$` or `@` reference, reopen its sheet to replace it. |
| Ctrl+G | Open the **Commands** sheet whenever the composer holds the keys, also during a turn, but not over a question or a permission request. A **Filter** field narrows the list; Enter runs the selected command. Your draft is untouched and waits above the sheet as `draft kept`. A command that opens a menu shows it over the draft, Esc goes back one level, and a choice returns to the composer. `/new` and `/resume` carry the draft into the conversation they open; a resumed conversation's own saved draft joins below it. |

### Conversation and work

| Key | Action |
| --- | --- |
| Esc | Close the open menu or panel, or leave prompt history; otherwise stop the running turn. Never scrolls. |
| Ctrl+C | Close a menu, leave prompt history, stop the running turn or, when idle, clear the draft. Never quits. |
| Ctrl+O | Expand or collapse the tool cards of the latest finished turn. Works only when idle with no panel open; the key is shown on that turn only. See [Tool activity](../usage/TOOL_ACTIVITY.md#expand-with-ctrlo). |
| Ctrl+Q | Quit, asking first while work is running. |
| F1 | Open help, also during a turn. |
| Page Up, Page Down, mouse wheel | Scroll the conversation. |
| Alt+Home, Alt+End | Jump to the beginning, or return to the latest output. |

On a Mac, Option types characters unless your terminal sends it as Alt: in Terminal,
turn on **Use Option as Meta key** in the profile's Keyboard settings; iTerm2 has a
similar setting per profile. Jecode never treats a typed character as a shortcut,
because Option+V is `@` on a Czech keyboard. Without a Home or End key, scrolling to the
end with the wheel or sending a message also returns to the latest output.

### Menus and panels

| Key | Action |
| --- | --- |
| Up, Down, Enter | Move and choose. In **Skills** and **Files**, Tab chooses too. |
| `1`–`9` | Choose directly in menus with up to nine options. |
| Typing | Filter longer lists. **Commands**, **Skills** and **Files** always filter. |
| Backspace | In **Files**, with an empty filter, go to the parent folder. |
| Left, Right | Switch tabs, such as **Commands** and **Keys** in `/help`. |
| Esc | Go back or close; first cancels a pending deletion mark. **Skills** and **Files** return the draft as it was when they opened. |
| Ctrl+D | In `/resume`, mark the selected conversation for deletion; in `/process`, ask to stop the selected process. Enter confirms both. In `/permissions` → **Remembered**, revoke the selected approval at once, with no confirmation. |
| `b` | In `/process`, when a call is waiting for the selected process, move it to the background: the call returns and the command keeps running. |

### Background process output

| Key | Action |
| --- | --- |
| Tab | Switch between **Output** and **Errors**. |
| Home, End | Jump to the start, or follow new output. |
| `i` | Type directly into an interactive process. |
| Ctrl+] | Leave direct input. Ctrl+C in direct input sends an interrupt to the process. |

Questions from the agent have their own keys; see
[Questions and plans](../usage/QUESTIONS_AND_PLANS.md#answer-questions).
