# Owned Unix process boundary. This shell becomes the target process; the
# guardian stays in its process group and observes a private owner pipe.
input_path=$1
shift
group=$$
case $1 in
    */*) program=$1 ;;
    *) program=$(type -P -- "$1") ;;
esac
if [ -z "$program" ] || [ ! -f "$program" ] || [ ! -x "$program" ]; then
    printf 'ERROR\n'
    exit 1
fi
shift
exec 3<&0
exec 0</dev/null
(
    exec 0<&3 3<&- 1>/dev/null 2>/dev/null
    IFS= read -r control || :
    if [ "$input_path" != /dev/null ]; then
        rm -f -- "$input_path"
        rmdir -- "${input_path%/*}"
    fi
    # The guardian keeps the group ID alive even after the target exits.
    builtin kill -KILL -- "-$group"
) &
exec 3<&-
exec 0<"$input_path" || exit 1
printf 'READY\n'
exec "$program" "$@"
