#ifndef JECODE_LINUX_CLIPBOARD_H
#define JECODE_LINUX_CLIPBOARD_H

#include <stddef.h>

#define JECODE_IMAGE_LIMIT (96u * 1024u * 1024u)

/* Return 0 on success, 3 for no image, 4 for desktop denial/unavailability,
 * 5 for invalid data or transfer failure, and 6 for missing imaging ABI. */
int jecode_x11_png(void);
int jecode_wayland_png(void);
int jecode_convert_image(void);
int jecode_write_all(const void *data, size_t length);
int jecode_copy_fd(int fd);
void *jecode_symbol(void *library, const char *name);

#endif
