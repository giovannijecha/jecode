# Commands and keys

The complete list of slash commands and keyboard controls in the TUI. `/help`
(or F1) shows the command list and common shortcuts inside Jecode. Plain-chat
commands are listed in
[Command line](../usage/CLI.md#plain-chat).

## Slash commands

Type `/` to see the commands grouped as below. **During a turn** says what
happens if you run a command while the agent is working: *now* runs it
immediately, *opens* shows the menu at once and applies conversation changes
from the next turn, *waits* queues it until the turn ends.

### Conversation

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/new` | Start a new conversation with your saved defaults. Alias: `/clear`. | Waits |
| `/resume [ID]` | Open a saved conversation from this folder, or delete one. See [Saved conversations](../usage/SESSIONS.md). | Waits |
| `/copy` | Copy the latest answer or one of its code blocks or quotes. See [Copy and export](../usage/COPY_AND_EXPORT.md). | Now |
| `/export` | Save the conversation as a JSON file in the working directory. | Waits |
| `/agents` | Inspect collaborators. See [Collaborating agents](../usage/COLLABORATION.md#watch-in-the-tui). | Now |
| `/ask` | Return to questions the agent is waiting on. | Now |
| `/todo` | Show the conversation's plan. See [Questions and plans](../usage/QUESTIONS_AND_PLANS.md#plans). | Now |
| `/exit` | Quit after pending work completes. Alias: `/quit`. | Waits |

### Model

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/model [ID]` | Choose the provider, model and effort for this conversation. | Opens |
| `/effort [NAME]` | Choose the reasoning effort for this conversation. | Opens |
| `/routing` | Choose an OpenRouter endpoint for this conversation. See [Configuration](../usage/CONFIGURATION.md#openrouter-endpoint-routing). | Opens |
| `/permissions [LEVEL]` | Choose Read, Workspace or Full for this conversation, or review remembered approvals. See [Permissions](../usage/PERMISSIONS.md). | Now |

### Workspace

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/init` | Have the model create or update the project's `AGENTS.md`. See [Project instructions](../usage/PROJECT_INSTRUCTIONS.md). | Waits |
| `/skills` | List your personal skills. See [Skills](../usage/SKILLS.md). | Opens |
| `/tmp [clean]` | Show, or clear, the conversation's temporary files. See [Temporary files](../usage/TEMPORARY.md). | Waits |
| `/checkpoints [COMMAND]` | List, inspect or restore checkpoints; `/checkpoints help` shows the commands. See [Checkpoints](../usage/CHECKPOINTS.md). | Waits |
| `/terminal [ID]` | Inspect, stop or type into background processes; `/terminal stop ID` stops one. See [Terminal interface](INTERFACE.md#background-processes). | Now |

### Jecode

| Command | Purpose | During a turn |
| --- | --- | --- |
| `/settings` | Change saved defaults and connect providers. Alias: `/setup`. See [Configuration](../usage/CONFIGURATION.md). | Opens |
| `/help` | Show commands and keys. | Opens |

An unknown command reports an error, with a suggestion when the name is close to
a real one.

## Keys

### Composer

| Key | Action |
| --- | --- |
| Enter | Send the draft, or queue it while the agent works. |
| Ctrl+J | Insert a newline. Shift+Enter and Alt+Enter also work when the terminal reports them. |
| Left, Right | Move by character. |
| Ctrl+Left, Ctrl+Right | Move by word. |
| Up, Down | Move between rows of the draft. In an empty composer, Up takes back the pending message. |
| Home, End, or Ctrl+A, Ctrl+E | Go to the start or end of the line. |
| Ctrl+Home, Ctrl+End | Go to the start or end of the draft. |
| Backspace, Delete | Delete a character, or a whole attachment. |
| Ctrl+Backspace, Ctrl+Delete, Ctrl+W | Delete a word, when the terminal reports the shortcut. |
| Ctrl+U | Delete to the start of the line. |
| Ctrl+P, Ctrl+N | Browse your last 50 sent messages; Esc returns to the draft. |
| Alt+V | Attach an image from the clipboard. Ctrl+V stays your terminal's text paste. |
| `/` | Open the command list; Up/Down select, Tab completes, Enter runs. |

### Conversation and work

| Key | Action |
| --- | --- |
| Esc | Close the open menu or panel, or leave prompt history; otherwise stop the running turn. Never scrolls. |
| Ctrl+C | Close a menu, leave prompt history, stop the running turn or, when idle, clear the draft. Never quits. |
| Ctrl+O | Expand or collapse the tool cards of the latest finished turn. Works only when idle with no panel open. See [Tool activity](TOOL_ACTIVITY.md#expand-with-ctrlo). |
| Ctrl+Q | Quit, asking first while work is running. |
| F1 | Open help, also during a turn. |
| Page Up, Page Down, mouse wheel | Scroll the conversation. |
| Alt+Home, Alt+End | Jump to the beginning, or return to the latest output. |

### Menus and panels

| Key | Action |
| --- | --- |
| Up, Down, Enter | Move and choose. |
| `1`–`9` | Choose directly in menus with up to eight options. |
| Typing | Filter longer lists. |
| Left, Right | Switch tabs, such as **Commands** and **Keys** in `/help`. |
| Esc | Go back or close; first cancels a pending deletion mark. |
| Ctrl+D | Mark the selected conversation for deletion in `/resume`, ask to stop a process in `/terminal`, or revoke an approval in `/permissions` → **Remembered**. Enter confirms. |

### Background process output

| Key | Action |
| --- | --- |
| Tab | Switch between **Output** and **Errors**. |
| Home, End | Jump to the start, or follow new output. |
| `i` | Type directly into an interactive process. |
| Ctrl+] | Leave direct input. Ctrl+C in direct input sends an interrupt to the process. |

Questions from the agent have their own keys; see
[Questions and plans](../usage/QUESTIONS_AND_PLANS.md#answer-questions).
