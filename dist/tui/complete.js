// Completing a slash command.
//
// A command the user cannot remember is a command that does not exist, and
// `/help` only helps someone who already knows to ask. So the menu appears
// while the line is being typed. Selection is state, not an edit: arrows move
// through the list without rewriting what the user typed.
import { COMMANDS } from "../commands.js";
/**
 * The commands a half-typed line still matches.
 *
 * Empty once the line carries an argument: at that point the user is naming a
 * model, not choosing a command, and a menu would be noise over the answer.
 */
export function matches(text) {
    if (!text.startsWith("/"))
        return [];
    const typed = text.slice(1);
    if (/\s/.test(typed))
        return [];
    return COMMANDS.filter((command) => command.name.startsWith(typed.toLowerCase()));
}
/**
 * The nth match of `prefix`, or undefined when there is nothing to complete.
 *
 * The prefix is what the user typed, not what tab last wrote: completing `/`
 * to `/help` would otherwise leave a line that matches only itself, and the
 * second tab would have nowhere to go.
 */
export function pick(prefix, index) {
    const list = matches(prefix);
    if (list.length === 0)
        return undefined;
    const at = ((index % list.length) + list.length) % list.length;
    return `/${list[at].name}`;
}
export function activate(text) {
    return matches(text).length === 0 ? undefined : { prefix: text, index: 0 };
}
export function move(completion, step) {
    const count = matches(completion.prefix).length;
    if (count === 0)
        return completion;
    return {
        ...completion,
        index: ((completion.index + step) % count + count) % count,
    };
}
export function selected(completion) {
    return pick(completion.prefix, completion.index);
}
/** Suggestions stay visible only while completion mode is active. */
export function options(completion) {
    return completion === undefined ? [] : matches(completion.prefix);
}
