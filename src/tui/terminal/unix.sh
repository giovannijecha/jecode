# Owned terminal-mode guardian. Input bytes are read by Rust, not by Bash.
# Opening /dev/tty avoids the different GNU/BSD stty device flags.
export LC_ALL=C
exec 3<>/dev/tty || exit 1
saved=$(stty -g <&3) || exit 1
restore() {
    trap - EXIT HUP INT TERM
    printf '\033[0m\033[?2026l\033[?1006l\033[?1000l\033[?2004l\033[?1049l\033[?25h' >&3
    stty "$saved" <&3
}
trap restore EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
stty raw -echo -ixon -ixoff opost onlcr min 0 time 1 <&3 || exit 1
printf '\033[?1049h\033[?2004h\033[?1000h\033[?1006h\033[?25l' >&3 || exit 1
printf 'READY\n'
# The owner holds this private pipe. Stop or owner death both restore modes.
IFS= read -r control
# EOF is the expected shutdown signal, not a guardian failure.
exit 0
