/* X11 CLIPBOARD selection, including INCR transfers. Only libX11's stable C
 * ABI is used; the server chooses whether image/png is offered. */
#define _POSIX_C_SOURCE 200809L
#include "linux_clipboard.h"
#include <dlfcn.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>

typedef unsigned long Xid;
typedef struct {
    int type;
    Xid serial;
    int send_event;
    void *display;
    Xid requestor, selection, target, property, time;
} SelectionEvent;
typedef struct {
    int type;
    Xid serial;
    int send_event;
    void *display;
    Xid window, atom, time;
    int state;
} PropertyEvent;
typedef union {
    int type;
    SelectionEvent selection;
    PropertyEvent property;
    long padding[24];
} XEvent;

typedef struct {
    void *library, *display;
    void *(*open)(const char *);
    int (*close)(void *);
    Xid (*root)(void *);
    Xid (*create)(void *, Xid, int, int, unsigned int, unsigned int, unsigned int, Xid, Xid);
    int (*destroy)(void *, Xid);
    Xid (*atom)(void *, const char *, int);
    int (*convert)(void *, Xid, Xid, Xid, Xid, Xid);
    int (*select)(void *, Xid, long);
    int (*pending)(void *);
    int (*next)(void *, XEvent *);
    int (*flush)(void *);
    int (*get)(void *, Xid, Xid, long, long, int, Xid, Xid *, int *, Xid *, Xid *, unsigned char **);
    int (*delete)(void *, Xid, Xid);
    int (*free_data)(void *);
} Xlib;

static int load(Xlib *x) {
    x->library = dlopen("libX11.so.6", RTLD_NOW | RTLD_LOCAL);
    if (!x->library) return 4;
#define API(field, symbol) do { x->field = jecode_symbol(x->library, symbol); if (!x->field) return 4; } while (0)
    API(open, "XOpenDisplay"); API(close, "XCloseDisplay");
    API(root, "XDefaultRootWindow"); API(create, "XCreateSimpleWindow");
    API(destroy, "XDestroyWindow"); API(atom, "XInternAtom");
    API(convert, "XConvertSelection"); API(select, "XSelectInput");
    API(pending, "XPending"); API(next, "XNextEvent");
    API(flush, "XFlush"); API(get, "XGetWindowProperty");
    API(delete, "XDeleteProperty"); API(free_data, "XFree");
#undef API
    return 0;
}

static long long millis(void) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (long long)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static int await_event(Xlib *x, XEvent *event, int type, Xid property, Xid window) {
    long long deadline = millis() + 18000;
    while (millis() < deadline) {
        while (x->pending(x->display)) {
            x->next(x->display, event);
            if (event->type == type &&
                (type == 31 || (event->property.window == window &&
                 event->property.atom == property && event->property.state == 0))) return 0;
        }
        struct timespec pause = {0, 10000000};
        nanosleep(&pause, NULL);
    }
    return 4;
}

static int read_property(Xlib *x, Xid window, Xid property, Xid png, Xid incr,
                         int *more, size_t *total) {
    Xid actual = 0, count = 0, after = 0;
    int format = 0;
    unsigned char *data = NULL;
    int status = x->get(x->display, window, property, 0,
                        JECODE_IMAGE_LIMIT / 4, 0, 0,
                        &actual, &format, &count, &after, &data);
    if (status || after || (actual != png && actual != incr && actual != 0) ||
        (actual == png && format != 8) ||
        (actual != incr && count > JECODE_IMAGE_LIMIT - *total)) {
        if (data) x->free_data(data);
        return 5;
    }
    if (actual == incr) {
        *more = 2;
    } else if (count) {
        if (jecode_write_all(data, (size_t)count)) status = 5;
        *total += (size_t)count;
        *more = 1;
    } else {
        *more = 0;
    }
    if (data) x->free_data(data);
    return status ? 5 : 0;
}

int jecode_x11_png(void) {
    Xlib x = {0};
    int result = load(&x);
    if (result) goto done;
    x.display = x.open(NULL);
    if (!x.display) { result = 4; goto done; }
    Xid window = x.create(x.display, x.root(x.display), 0, 0, 1, 1, 0, 0, 0);
    if (!window) { result = 4; goto done; }
    Xid clipboard = x.atom(x.display, "CLIPBOARD", 0);
    Xid png = x.atom(x.display, "image/png", 0);
    Xid property = x.atom(x.display, "JECODE_CLIPBOARD_PNG", 0);
    Xid incr = x.atom(x.display, "INCR", 0);
    x.select(x.display, window, 1L << 22);
    x.convert(x.display, clipboard, png, property, window, 0);
    x.flush(x.display);
    XEvent event;
    result = await_event(&x, &event, 31, property, window);
    if (result) goto close_window;
    if (!event.selection.property) { result = 3; goto close_window; }
    int more = 0;
    size_t total = 0;
    result = read_property(&x, window, property, png, incr, &more, &total);
    if (result) goto close_window;
    if (more == 2) {
        x.delete(x.display, window, property);
        x.flush(x.display);
        size_t rounds = 0;
        do {
            result = await_event(&x, &event, 28, property, window);
            if (result) break;
            result = read_property(&x, window, property, png, incr, &more, &total);
            x.delete(x.display, window, property);
            x.flush(x.display);
        } while (!result && more && ++rounds < 2048);
        if (rounds == 2048) result = 5;
        if (!total && !result) result = 3;
    } else {
        result = more ? 0 : 3;
    }
close_window:
    x.destroy(x.display, window);
done:
    if (x.display) x.close(x.display);
    if (x.library) dlclose(x.library);
    return result;
}
