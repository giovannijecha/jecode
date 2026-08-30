// Mutable screen state shared by the shell, input, and foreground workflows.
import * as edit from "./editor.js";
export function appState() {
    return {
        blocks: [],
        editor: edit.EMPTY,
        scroll: 0,
        follow: true,
        unseen: 0,
        lastMaxScroll: 0,
        past: [],
        recall: -1,
        draft: "",
        spin: 0,
        closeWhenIdle: false,
    };
}
