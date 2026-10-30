/* Owned Linux clipboard and image adapter. Desktop libraries are loaded at
 * runtime; no helper application, protocol generator or development header is
 * needed on the user's machine. Binary data travels on stdin/stdout only. */
#include "linux_clipboard.h"
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

void *jecode_symbol(void *library, const char *name) {
    return library ? dlsym(library, name) : NULL;
}

int jecode_write_all(const void *data, size_t length) {
    const unsigned char *bytes = data;
    while (length) {
        ssize_t count = write(STDOUT_FILENO, bytes, length);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) return 5;
        bytes += (size_t)count;
        length -= (size_t)count;
    }
    return 0;
}

int jecode_copy_fd(int fd) {
    unsigned char buffer[65536];
    size_t total = 0;
    for (;;) {
        ssize_t count = read(fd, buffer, sizeof(buffer));
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) return 5;
        if (count == 0) return total ? 0 : 3;
        if ((size_t)count > JECODE_IMAGE_LIMIT - total) return 5;
        total += (size_t)count;
        if (jecode_write_all(buffer, (size_t)count)) return 5;
    }
}

int main(int argc, char **argv) {
    if (argc != 2) return 5;
    if (!strcmp(argv[1], "convert")) return jecode_convert_image();
    if (strcmp(argv[1], "clipboard")) return 5;
    /* Compositors without data-control (e.g. GNOME) report 4 before writing
     * anything, so XWayland can still serve the clipboard. */
    int result = getenv("WAYLAND_DISPLAY") ? jecode_wayland_png() : 4;
    if (result == 4 && getenv("DISPLAY")) result = jecode_x11_png();
    return result;
}
