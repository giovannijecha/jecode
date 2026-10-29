/* Wayland clipboard through compositor-advertised data-control. The stable
 * ext protocol is preferred; the older wlr protocol has the same first
 * request/event signatures. No keyboard-focused surface is required. */
#include "linux_clipboard.h"
#include <dlfcn.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>

struct wl_message { const char *name, *signature; const struct wl_interface **types; };
struct wl_interface {
    const char *name;
    int version, method_count;
    const struct wl_message *methods;
    int event_count;
    const struct wl_message *events;
};

static const struct wl_interface offer_interface;
static const struct wl_interface device_interface;
static const struct wl_interface *offer_type[] = {&offer_interface};
static const struct wl_message offer_methods[] = {
    {"receive", "sh", NULL}, {"destroy", "", NULL}
};
static const struct wl_message offer_events[] = {{"offer", "s", NULL}};
static const struct wl_interface offer_interface = {
    "ext_data_control_offer_v1", 1, 2, offer_methods, 1, offer_events
};
static const struct wl_message device_methods[] = {
    {"set_selection", "?o", NULL}, {"destroy", "", NULL},
    {"set_primary_selection", "?o", NULL}
};
static const struct wl_message device_events[] = {
    {"data_offer", "n", offer_type}, {"selection", "?o", offer_type},
    {"finished", "", NULL}, {"primary_selection", "?o", offer_type}
};
static const struct wl_interface device_interface = {
    "ext_data_control_device_v1", 1, 3, device_methods, 4, device_events
};
static const struct wl_message manager_methods[] = {
    {"create_data_source", "n", NULL},
    {"get_data_device", "no", NULL}, {"destroy", "", NULL}
};
static const struct wl_interface manager_interface = {
    "ext_data_control_manager_v1", 1, 3, manager_methods, 0, NULL
};

typedef struct Context Context;
typedef struct { void *proxy; int png; } Offer;
struct Context {
    void *library, *display, *registry, *seat, *manager, *device;
    const struct wl_interface *registry_interface, *seat_interface;
    struct wl_interface manager_type, device_type, offer_type;
    struct wl_message manager_method_types[3];
    const struct wl_interface *manager_device_types[2];
    struct wl_message device_event_types[4];
    const struct wl_interface *offer_event_type[1];
    uint32_t seat_name, manager_name, manager_version;
    int use_wlr, selected_seen, finished;
    Offer offers[16];
    size_t offer_count;
    Offer *selected;
    void *(*connect)(const char *);
    void (*disconnect)(void *);
    int (*roundtrip)(void *);
    int (*flush)(void *);
    int (*listener)(void *, void (**)(void), void *);
    void *(*marshal)(void *, uint32_t, const struct wl_interface *, uint32_t, uint32_t, ...);
    uint32_t (*version)(void *);
    void (*destroy_proxy)(void *);
};

static void offer_mime(void *data, void *proxy, const char *mime) {
    Offer *offer = data;
    (void)proxy;
    if (!strcmp(mime, "image/png")) offer->png = 1;
}

static void (*offer_listeners[])(void) = {(void (*)(void))offer_mime};

static void data_offer(void *data, void *device, void *proxy) {
    Context *ctx = data;
    (void)device;
    if (ctx->offer_count == 16) return;
    Offer *offer = &ctx->offers[ctx->offer_count++];
    offer->proxy = proxy;
    ctx->listener(proxy, offer_listeners, offer);
}
static void selection(void *data, void *device, void *proxy) {
    Context *ctx = data;
    (void)device;
    ctx->selected_seen = 1;
    ctx->selected = NULL;
    for (size_t i = 0; i < ctx->offer_count; i++) {
        if (ctx->offers[i].proxy == proxy) ctx->selected = &ctx->offers[i];
    }
}
static void finished(void *data, void *device) {
    Context *ctx = data;
    (void)device;
    ctx->finished = 1;
}
static void primary(void *data, void *device, void *proxy) {
    (void)data; (void)device; (void)proxy;
}
static void (*device_listeners[])(void) = {
    (void (*)(void))data_offer, (void (*)(void))selection,
    (void (*)(void))finished, (void (*)(void))primary
};

static void global(void *data, void *registry, uint32_t name,
                   const char *interface, uint32_t version) {
    Context *ctx = data;
    (void)registry;
    if (!strcmp(interface, "wl_seat") && !ctx->seat_name) ctx->seat_name = name;
    if (!strcmp(interface, "ext_data_control_manager_v1")) {
        ctx->manager_name = name; ctx->manager_version = version; ctx->use_wlr = 0;
    } else if (!strcmp(interface, "zwlr_data_control_manager_v1") && !ctx->manager_name) {
        ctx->manager_name = name; ctx->manager_version = version; ctx->use_wlr = 1;
    }
}
static void global_remove(void *data, void *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}
static void (*registry_listeners[])(void) = {
    (void (*)(void))global, (void (*)(void))global_remove
};

static int load(Context *ctx) {
    ctx->library = dlopen("libwayland-client.so.0", RTLD_NOW | RTLD_LOCAL);
    if (!ctx->library) return 4;
#define API(field, symbol) do { ctx->field = jecode_symbol(ctx->library, symbol); if (!ctx->field) return 4; } while (0)
    API(connect, "wl_display_connect"); API(disconnect, "wl_display_disconnect");
    API(roundtrip, "wl_display_roundtrip"); API(flush, "wl_display_flush");
    API(listener, "wl_proxy_add_listener"); API(marshal, "wl_proxy_marshal_flags");
    API(version, "wl_proxy_get_version"); API(destroy_proxy, "wl_proxy_destroy");
#undef API
    ctx->registry_interface = jecode_symbol(ctx->library, "wl_registry_interface");
    ctx->seat_interface = jecode_symbol(ctx->library, "wl_seat_interface");
    return ctx->registry_interface && ctx->seat_interface ? 0 : 4;
}

int jecode_wayland_png(void) {
    Context ctx = {0};
    int result = load(&ctx);
    if (result) goto done;
    ctx.display = ctx.connect(NULL);
    if (!ctx.display) { result = 4; goto done; }
    ctx.registry = ctx.marshal(ctx.display, 1, ctx.registry_interface, 1, 0, NULL);
    if (!ctx.registry || ctx.listener(ctx.registry, registry_listeners, &ctx) ||
        ctx.roundtrip(ctx.display) < 0) { result = 4; goto done; }
    if (!ctx.seat_name || !ctx.manager_name || !ctx.manager_version) {
        result = 4; goto done;
    }
    ctx.manager_type = manager_interface;
    ctx.device_type = device_interface;
    ctx.offer_type = offer_interface;
    if (ctx.use_wlr) {
        ctx.manager_type.name = "zwlr_data_control_manager_v1";
        ctx.device_type.name = "zwlr_data_control_device_v1";
        ctx.offer_type.name = "zwlr_data_control_offer_v1";
        ctx.manager_type.version = ctx.device_type.version = 2;
    }
    memcpy(ctx.manager_method_types, manager_methods, sizeof(manager_methods));
    ctx.manager_device_types[0] = &ctx.device_type;
    ctx.manager_device_types[1] = ctx.seat_interface;
    ctx.manager_method_types[1].types = ctx.manager_device_types;
    ctx.manager_type.methods = ctx.manager_method_types;
    memcpy(ctx.device_event_types, device_events, sizeof(device_events));
    ctx.offer_event_type[0] = &ctx.offer_type;
    ctx.device_event_types[0].types = ctx.offer_event_type;
    ctx.device_event_types[1].types = ctx.offer_event_type;
    ctx.device_event_types[3].types = ctx.offer_event_type;
    if (ctx.use_wlr) ctx.device_event_types[3].signature = "2?o";
    ctx.device_type.events = ctx.device_event_types;
    uint32_t version = ctx.manager_version < (uint32_t)ctx.manager_type.version
                     ? ctx.manager_version : (uint32_t)ctx.manager_type.version;
    ctx.seat = ctx.marshal(ctx.registry, 0, ctx.seat_interface, 1, 0,
                           ctx.seat_name, ctx.seat_interface->name, 1, NULL);
    ctx.manager = ctx.marshal(ctx.registry, 0, &ctx.manager_type, version, 0,
                              ctx.manager_name, ctx.manager_type.name, version, NULL);
    if (!ctx.seat || !ctx.manager) { result = 4; goto done; }
    ctx.device = ctx.marshal(ctx.manager, 1, &ctx.device_type, version, 0,
                             NULL, ctx.seat);
    if (!ctx.device || ctx.listener(ctx.device, device_listeners, &ctx) ||
        ctx.roundtrip(ctx.display) < 0 || ctx.finished || !ctx.selected_seen) {
        result = 4; goto done;
    }
    if (!ctx.selected || !ctx.selected->png) { result = 3; goto done; }
    int pipes[2];
    if (pipe(pipes)) { result = 5; goto done; }
    ctx.marshal(ctx.selected->proxy, 0, NULL, 1, 0, "image/png", pipes[1]);
    result = ctx.flush(ctx.display) < 0 ? 4 : 0;
    close(pipes[1]);
    if (!result) result = jecode_copy_fd(pipes[0]);
    close(pipes[0]);
done:
    /* The connection owns its proxies and closes them together. */
    if (ctx.display) ctx.disconnect(ctx.display);
    if (ctx.library) dlclose(ctx.library);
    return result;
}
