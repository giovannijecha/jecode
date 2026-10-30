/* GdkPixbuf is a desktop imaging library, loaded through its stable C ABI.
 * Decode, bound geometry, save PNG, and use JPEG only when it is smaller. */
#include "linux_clipboard.h"
#include <dlfcn.h>
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <unistd.h>

typedef struct {
    void *pixbuf, *gobject, *glib;
    void *(*loader_new)(void);
    int (*loader_write)(void *, const unsigned char *, size_t, void **);
    int (*loader_close)(void *, void **);
    void *(*loader_pixbuf)(void *);
    int (*width)(void *), (*height)(void *), (*alpha)(void *);
    void *(*scale)(void *, int, int, int);
    void *(*new_pixbuf)(int, int, int, int, int);
    void (*fill)(void *, uint32_t);
    void (*composite)(void *, void *, int, int, int, int,
                      double, double, double, double, int, int);
    int (*save)(void *, char **, size_t *, const char *, char **, char **, void **);
    void (*unref)(void *);
    void (*free_data)(void *);
} Images;

static int load(Images *g) {
    g->pixbuf = dlopen("libgdk_pixbuf-2.0.so.0", RTLD_NOW | RTLD_LOCAL);
    g->gobject = dlopen("libgobject-2.0.so.0", RTLD_NOW | RTLD_LOCAL);
    g->glib = dlopen("libglib-2.0.so.0", RTLD_NOW | RTLD_LOCAL);
    if (!g->pixbuf || !g->gobject || !g->glib) return 6;
#define API(field, library, symbol) do { g->field = jecode_symbol(g->library, symbol); if (!g->field) return 6; } while (0)
    API(loader_new, pixbuf, "gdk_pixbuf_loader_new");
    API(loader_write, pixbuf, "gdk_pixbuf_loader_write");
    API(loader_close, pixbuf, "gdk_pixbuf_loader_close");
    API(loader_pixbuf, pixbuf, "gdk_pixbuf_loader_get_pixbuf");
    API(width, pixbuf, "gdk_pixbuf_get_width");
    API(height, pixbuf, "gdk_pixbuf_get_height");
    API(alpha, pixbuf, "gdk_pixbuf_get_has_alpha");
    API(scale, pixbuf, "gdk_pixbuf_scale_simple");
    API(new_pixbuf, pixbuf, "gdk_pixbuf_new");
    API(fill, pixbuf, "gdk_pixbuf_fill");
    API(composite, pixbuf, "gdk_pixbuf_composite");
    API(save, pixbuf, "gdk_pixbuf_save_to_bufferv");
    API(unref, gobject, "g_object_unref");
    API(free_data, glib, "g_free");
#undef API
    return 0;
}

static void unload(Images *g) {
    if (g->glib) dlclose(g->glib);
    if (g->gobject) dlclose(g->gobject);
    if (g->pixbuf) dlclose(g->pixbuf);
}

static unsigned char *read_input(size_t *length) {
    size_t capacity = 65536;
    unsigned char *bytes = malloc(capacity);
    if (!bytes) return NULL;
    *length = 0;
    for (;;) {
        if (*length == capacity) {
            if (capacity >= JECODE_IMAGE_LIMIT) break;
            size_t next = capacity * 2;
            if (next > JECODE_IMAGE_LIMIT) next = JECODE_IMAGE_LIMIT;
            unsigned char *grown = realloc(bytes, next);
            if (!grown) { free(bytes); return NULL; }
            bytes = grown; capacity = next;
        }
        ssize_t count = read(STDIN_FILENO, bytes + *length, capacity - *length);
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) { free(bytes); return NULL; }
        if (!count) return bytes;
        *length += (size_t)count;
    }
    free(bytes);
    return NULL;
}

int jecode_convert_image(void) {
    Images g = {0};
    int result = load(&g);
    if (result) { unload(&g); return result; }
    size_t input_size = 0;
    unsigned char *input = read_input(&input_size);
    void *loader = NULL, *source = NULL, *scaled = NULL, *flat = NULL;
    char *png = NULL, *jpeg = NULL;
    size_t png_size = 0, jpeg_size = 0;
    if (!input || !input_size) { result = 5; goto done; }
    loader = g.loader_new();
    if (!loader || !g.loader_write(loader, input, input_size, NULL) ||
        !g.loader_close(loader, NULL)) { result = 5; goto done; }
    source = g.loader_pixbuf(loader);
    if (!source) { result = 5; goto done; }
    int width = g.width(source), height = g.height(source);
    if (width <= 0 || height <= 0) { result = 5; goto done; }
    int longest = width > height ? width : height;
    double ratio = longest > 2048 ? 2048.0 / longest : 1.0;
    int out_width = (int)(width * ratio + 0.5);
    int out_height = (int)(height * ratio + 0.5);
    if (out_width < 1) out_width = 1;
    if (out_height < 1) out_height = 1;
    scaled = ratio < 1.0 ? g.scale(source, out_width, out_height, 3) : source;
    if (!scaled || !g.save(scaled, &png, &png_size, "png", NULL, NULL, NULL) ||
        png_size > JECODE_IMAGE_LIMIT - 4) { result = 5; goto done; }
    if (png_size > 512u * 1024u) {
        void *jpeg_source = scaled;
        if (g.alpha(scaled)) {
            flat = g.new_pixbuf(0, 0, 8, out_width, out_height);
            if (!flat) goto write_png;
            g.fill(flat, 0xffffffffu);
            g.composite(scaled, flat, 0, 0, out_width, out_height,
                        0.0, 0.0, 1.0, 1.0, 2, 255);
            jpeg_source = flat;
        }
        char *keys[] = {"quality", NULL};
        char *values[] = {"85", NULL};
        if (g.save(jpeg_source, &jpeg, &jpeg_size, "jpeg", keys, values, NULL) &&
            jpeg_size < png_size && jpeg_size <= JECODE_IMAGE_LIMIT - 4) {
            result = jecode_write_all("jpg\n", 4);
            if (!result) result = jecode_write_all(jpeg, jpeg_size);
            goto done;
        }
    }
write_png:
    result = jecode_write_all("png\n", 4);
    if (!result) result = jecode_write_all(png, png_size);
done:
    if (jpeg) g.free_data(jpeg);
    if (png) g.free_data(png);
    if (flat) g.unref(flat);
    if (scaled && scaled != source) g.unref(scaled);
    if (loader) g.unref(loader);
    free(input);
    unload(&g);
    return result;
}
