/* Step ④a probe. Not the product helper.
   Parent: GtkWindow + GtkSocket. Helper: GtkPlug + WebKitWebView.
   GDK_BACKEND must already be x11. WebKit must be 2.28.1 or this exits. */
#include <gtk/gtk.h>
#include <gtk/gtkx.h>
#include <gdk/gdkx.h>
#include <webkit2/webkit2.h>
#include <jsc/jsc.h>
#include <X11/keysym.h>
#include <X11/extensions/XTest.h>

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>

static GtkWidget *g_plug;
static GtkWidget *g_entry;
static GtkWidget *g_web;
static int g_crashed;

static void say(const char *text) {
  fputs(text, stdout);
  fputc('\n', stdout);
  fflush(stdout);
}

static int webkit_is_2281(void) {
  unsigned maj = webkit_get_major_version();
  unsigned min = webkit_get_minor_version();
  unsigned mic = webkit_get_micro_version();
  printf("WEBKIT %u.%u.%u\n", maj, min, mic);
  fflush(stdout);
  return maj == 2 && min == 28 && mic == 1;
}

static void print_mapped_webkit(void) {
  FILE *f = fopen("/proc/self/maps", "r");
  char line[1024];
  if (!f) {
    say("MAPPED unreadable");
    return;
  }
  while (fgets(line, sizeof line, f)) {
    char *path = strchr(line, '/');
    if (!path || !strstr(path, "libwebkit2gtk")) continue;
    path[strcspn(path, "\n")] = 0;
    printf("MAPPED %s\n", path);
    fflush(stdout);
    fclose(f);
    return;
  }
  fclose(f);
  say("MAPPED none");
}

static int has_non_ascii(const char *s) {
  if (!s) return 0;
  for (; *s; s++) {
    if ((unsigned char)*s >= 0x80) return 1;
  }
  return 0;
}

static const char *page_html =
    "<!DOCTYPE html><html><head><meta charset=\"utf-8\">"
    "<title>boot</title><style>"
    "html,body{margin:0;height:100%;}"
    "#q{box-sizing:border-box;width:100%;height:100%;font-size:24px;}"
    "</style></head><body>"
    "<input id=\"q\" autofocus>"
    "<script>"
    "window.__eb={mouse:'',key:'',text:'',comp:'',dpr:String(window.devicePixelRatio),"
    "roCtor:(typeof ResizeObserver==='function'?'ok':'missing'),ro:'',iw:String(window.innerWidth)};"
    "window.__eb.dump=function(){"
    "return 'mouse='+this.mouse+' key='+this.key+' text='+encodeURIComponent(this.text)+"
    "' comp='+encodeURIComponent(this.comp)+' dpr='+this.dpr+' roCtor='+this.roCtor+"
    "' ro='+this.ro+' iw='+window.innerWidth;"
    "};"
    "document.addEventListener('mousedown',function(e){"
    "window.__eb.mouse=e.clientX+','+e.clientY;"
    "},true);"
    "document.addEventListener('keydown',function(e){"
    "window.__eb.key=(window.__eb.key+e.key).slice(-32);"
    "},true);"
    "var q=document.getElementById('q');"
    "q.addEventListener('input',function(){window.__eb.text=q.value;});"
    "q.addEventListener('compositionupdate',function(e){window.__eb.comp=e.data||'';});"
    "q.addEventListener('compositionend',function(e){window.__eb.comp=e.data||window.__eb.comp;});"
    "if (typeof ResizeObserver==='function') {"
    "new ResizeObserver(function(entries){"
    "var r=entries[0].contentRect;"
    "window.__eb.ro=Math.round(r.width)+'x'+Math.round(r.height);"
    "}).observe(document.documentElement);"
    "}"
    "</script></body></html>";

static gboolean helper_exit_nonzero(gpointer data) {
  (void)data;
  say("HELPER_ALIVE");
  exit(3);
}

static gboolean on_crashed(WebKitWebView *web, gpointer data) {
  (void)web;
  (void)data;
  g_crashed = 1;
  say("SIGNAL web-process-crashed");
  g_timeout_add(300, helper_exit_nonzero, NULL);
  return TRUE;
}

static void on_terminated(WebKitWebView *web, WebKitWebProcessTerminationReason reason, gpointer data) {
  (void)web;
  (void)data;
  printf("SIGNAL web-process-terminated reason=%d\n", (int)reason);
  fflush(stdout);
}

static void on_load(WebKitWebView *web, WebKitLoadEvent ev, gpointer data) {
  (void)web;
  (void)data;
  if (ev == WEBKIT_LOAD_FINISHED) say("LOADED");
}

static gboolean on_load_failed(WebKitWebView *web, WebKitLoadEvent ev, char *uri, GError *error, gpointer data) {
  (void)web;
  (void)ev;
  (void)uri;
  (void)data;
  printf("LOAD_FAILED %s\n", error && error->message ? error->message : "?");
  fflush(stdout);
  return FALSE;
}

static void on_js_done(GObject *src, GAsyncResult *res, gpointer data) {
  GError *err = NULL;
  WebKitJavascriptResult *js;
  JSCValue *value;
  char *text;
  (void)data;
  js = webkit_web_view_run_javascript_finish(WEBKIT_WEB_VIEW(src), res, &err);
  if (!js) {
    printf("JS_FAIL %s\n", err && err->message ? err->message : "?");
    fflush(stdout);
    if (err) g_error_free(err);
    return;
  }
  value = webkit_javascript_result_get_js_value(js);
  text = value ? jsc_value_to_string(value) : NULL;
  printf("JS %s\n", text ? text : "");
  fflush(stdout);
  g_free(text);
  webkit_javascript_result_unref(js);
}

static void on_entry_insert(GtkEditable *editable, gchar *text, gint len, gint *position, gpointer data) {
  (void)editable;
  (void)position;
  (void)data;
  if (!text) return;
  if (len < 0) len = (gint)strlen(text);
  printf("ENTRY_INSERT %.*s\n", (int)len, text);
  fflush(stdout);
}

static void helper_command(const char *line) {
  if (strcmp(line, "focus-entry") == 0) {
    gtk_widget_grab_focus(g_entry);
    say("FOCUS entry");
    return;
  }
  if (strcmp(line, "focus-web") == 0) {
    gtk_widget_grab_focus(g_web);
    say("FOCUS web");
    return;
  }
  if (strcmp(line, "entry") == 0) {
    printf("ENTRY %s\n", gtk_entry_get_text(GTK_ENTRY(g_entry)));
    fflush(stdout);
    return;
  }
  if (strcmp(line, "scale") == 0) {
    printf("SCALE web=%d entry=%d alloc=%dx%d\n",
           gtk_widget_get_scale_factor(g_web),
           gtk_widget_get_scale_factor(g_entry),
           gtk_widget_get_allocated_width(g_plug),
           gtk_widget_get_allocated_height(g_plug));
    fflush(stdout);
    return;
  }
  if (strncmp(line, "snap ", 5) == 0) {
    char script[256];
    snprintf(script, sizeof script,
             "window.__eb.dump ? ('snap=%s '+window.__eb.dump()) : 'snap=%s missing'",
             line + 5, line + 5);
    webkit_web_view_run_javascript(WEBKIT_WEB_VIEW(g_web), script, NULL, on_js_done, NULL);
  }
}

static gboolean on_helper_stdin(GIOChannel *channel, GIOCondition cond, gpointer data) {
  char *line = NULL;
  gsize len = 0;
  GError *err = NULL;
  (void)data;
  if (cond & (G_IO_HUP | G_IO_ERR)) return FALSE;
  if (g_io_channel_read_line(channel, &line, &len, NULL, &err) != G_IO_STATUS_NORMAL) {
    if (err) g_error_free(err);
    return FALSE;
  }
  if (line) {
    g_strchomp(line);
    helper_command(line);
    g_free(line);
  }
  return TRUE;
}

static int helper_main(void) {
  GtkWidget *box;
  GIOChannel *channel;
  if (!gtk_init_check(NULL, NULL)) {
    say("gtk_init_check FAILED");
    return 2;
  }
  if (!webkit_is_2281()) {
    say("REFUSE webkit is not 2.28.1");
    return 2;
  }
  print_mapped_webkit();
  printf("display_type=%s\n", G_OBJECT_TYPE_NAME(gdk_display_get_default()));
  printf("env GDK_BACKEND=%s GDK_SCALE=%s GTK_IM_MODULE=%s XMODIFIERS=%s\n",
         getenv("GDK_BACKEND") ? getenv("GDK_BACKEND") : "",
         getenv("GDK_SCALE") ? getenv("GDK_SCALE") : "",
         getenv("GTK_IM_MODULE") ? getenv("GTK_IM_MODULE") : "",
         getenv("XMODIFIERS") ? getenv("XMODIFIERS") : "");
  fflush(stdout);

  g_plug = gtk_plug_new(0);
  box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
  g_entry = gtk_entry_new();
  gtk_entry_set_placeholder_text(GTK_ENTRY(g_entry), "entry");
  g_signal_connect(g_entry, "insert-text", G_CALLBACK(on_entry_insert), NULL);
  g_web = webkit_web_view_new();
  gtk_box_pack_start(GTK_BOX(box), g_entry, FALSE, FALSE, 0);
  gtk_box_pack_start(GTK_BOX(box), g_web, TRUE, TRUE, 0);
  gtk_container_add(GTK_CONTAINER(g_plug), box);
  g_signal_connect(g_web, "load-changed", G_CALLBACK(on_load), NULL);
  g_signal_connect(g_web, "load-failed", G_CALLBACK(on_load_failed), NULL);
  g_signal_connect(g_web, "web-process-crashed", G_CALLBACK(on_crashed), NULL);
  g_signal_connect(g_web, "web-process-terminated", G_CALLBACK(on_terminated), NULL);
  gtk_widget_show_all(g_plug);
  printf("PLUG %lu\n", (unsigned long)gtk_plug_get_id(GTK_PLUG(g_plug)));
  fflush(stdout);
  webkit_web_view_load_html(WEBKIT_WEB_VIEW(g_web), page_html, "file:///tmp/eb-probe/");

  channel = g_io_channel_unix_new(STDIN_FILENO);
  g_io_channel_set_encoding(channel, NULL, NULL);
  g_io_channel_set_flags(channel, G_IO_FLAG_NONBLOCK, NULL);
  g_io_add_watch(channel, G_IO_IN | G_IO_HUP | G_IO_ERR, on_helper_stdin, NULL);
  gtk_main();
  return g_crashed ? 3 : 0;
}

typedef struct {
  GPid helper;
  int helper_in;
  unsigned long plug_id;
  int added;
  int trap;
  int loaded;
  int phase;
  gint64 phase_at;
  int snap;
  char js[2048];
  char entry[1024];
  char ro_before[64];
  int saw_crashed;
  int saw_terminated;
  int alive_at_crash;
  int helper_status;
  int helper_reaped;
  int xtest;
  GtkWidget *window;
  GtkWidget *socket;
  Display *dpy;
  char detail_mouse[160];
  char detail_key[160];
  char detail_im_entry[512];
  char detail_im_web[512];
  char detail_hidpi[200];
  char detail_ro[160];
  char detail_crash[160];
  char detail_adopt[200];
  char saved_mouse[128];
  char saved_key[128];
  char saved_text[256];
  char saved_comp[256];
  char saved_dpr[64];
  char saved_ro_after[64];
  char saved_ro_ctor[32];
  char entry_inserts[512];
  unsigned int socket_w;
  unsigned int plug_w;
  unsigned int alloc_before_resize;
  int plug_window_live;
  int saved_scale;
  int ro_resnap;
  int entry_fresh;
  pid_t web_pid;
  int result_code;
} Parent;

static Parent g_parent;

static void parent_send(const char *line) {
  char buf[256];
  int n;
  if (g_parent.helper_in < 0) return;
  n = snprintf(buf, sizeof buf, "%s\n", line);
  if (n > 0) {
    ssize_t w = write(g_parent.helper_in, buf, (size_t)n);
    (void)w;
  }
}

static void remember_js(const char *line) {
  snprintf(g_parent.js, sizeof g_parent.js, "%s", line);
}

static const char *js_field(const char *key) {
  static char value[256];
  char needle[64];
  const char *hit;
  size_t i = 0;
  snprintf(needle, sizeof needle, "%s=", key);
  hit = strstr(g_parent.js, needle);
  if (!hit) return "";
  hit += strlen(needle);
  while (hit[i] && hit[i] != ' ' && i + 1 < sizeof value) {
    value[i] = hit[i];
    i++;
  }
  value[i] = 0;
  return value;
}

static void click_window(Window win, int x, int y) {
  Window child = 0;
  int rx = 0;
  int ry = 0;
  XTranslateCoordinates(g_parent.dpy, win, DefaultRootWindow(g_parent.dpy), x, y, &rx, &ry, &child);
  XTestFakeMotionEvent(g_parent.dpy, -1, rx, ry, CurrentTime);
  XFlush(g_parent.dpy);
  g_usleep(40000);
  XTestFakeButtonEvent(g_parent.dpy, 1, True, CurrentTime);
  XFlush(g_parent.dpy);
  g_usleep(30000);
  XTestFakeButtonEvent(g_parent.dpy, 1, False, CurrentTime);
  XFlush(g_parent.dpy);
  g_usleep(40000);
}

static void tap_key(KeySym ks) {
  KeyCode code = XKeysymToKeycode(g_parent.dpy, ks);
  if (!code) {
    printf("KEY_MISSING %lu\n", (unsigned long)ks);
    fflush(stdout);
    return;
  }
  XTestFakeKeyEvent(g_parent.dpy, code, True, CurrentTime);
  XFlush(g_parent.dpy);
  g_usleep(25000);
  XTestFakeKeyEvent(g_parent.dpy, code, False, CurrentTime);
  XFlush(g_parent.dpy);
  g_usleep(25000);
}

static void type_letters(void) {
  tap_key(XK_z);
  tap_key(XK_h);
  tap_key(XK_o);
  tap_key(XK_n);
  tap_key(XK_g);
}

static Window socket_xid(void) {
  GdkWindow *window = gtk_widget_get_window(g_parent.socket);
  if (!window) return 0;
  return gdk_x11_window_get_xid(window);
}

static unsigned int socket_width(void) {
  Window win = socket_xid();
  Window root = 0;
  int x = 0;
  int y = 0;
  unsigned int w = 0;
  unsigned int h = 0;
  unsigned int bw = 0;
  unsigned int depth = 0;
  if (!win || !g_parent.dpy) return 0;
  if (!XGetGeometry(g_parent.dpy, win, &root, &x, &y, &w, &h, &bw, &depth)) return 0;
  return w;
}

static void log_geometry(const char *tag) {
  Window win = socket_xid();
  Window root = 0;
  int x = 0;
  int y = 0;
  unsigned int w = 0;
  unsigned int h = 0;
  unsigned int bw = 0;
  unsigned int depth = 0;
  if (!win) {
    printf("GEOM %s socket-xid-missing\n", tag);
    fflush(stdout);
    return;
  }
  XGetGeometry(g_parent.dpy, win, &root, &x, &y, &w, &h, &bw, &depth);
  printf("GEOM %s socket=%ux%u scale=%d plug=%lu\n", tag, w, h,
         gtk_widget_get_scale_factor(g_parent.socket), g_parent.plug_id);
  fflush(stdout);
  if (strcmp(tag, "initial") == 0) {
    g_parent.socket_w = w;
    g_parent.saved_scale = gtk_widget_get_scale_factor(g_parent.socket);
  }
  if (g_parent.plug_id) {
    unsigned int pw = 0;
    unsigned int ph = 0;
    if (XGetGeometry(g_parent.dpy, (Window)g_parent.plug_id, &root, &x, &y, &pw, &ph, &bw, &depth)) {
      printf("GEOM %s plug=%ux%u\n", tag, pw, ph);
      fflush(stdout);
      if (strcmp(tag, "initial") == 0) g_parent.plug_w = pw;
    }
  }
}

static void fcitx_cmd(const char *command, const char *tag) {
  char *out = NULL;
  char *err = NULL;
  int status = -1;
  GError *gerr = NULL;
  if (!g_spawn_command_line_sync(command, &out, &err, &status, &gerr)) {
    printf("FCITX %s spawn-failed %s\n", tag, gerr && gerr->message ? gerr->message : "?");
    fflush(stdout);
    if (gerr) g_error_free(gerr);
    return;
  }
  if (out) g_strchomp(out);
  printf("FCITX %s status=%d out=%s err=%s\n", tag, status, out ? out : "", err ? err : "");
  fflush(stdout);
  g_free(out);
  g_free(err);
}

static int descendant_is_web_process(pid_t pid) {
  char path[64];
  char buf[1024];
  FILE *f;
  size_t n;
  size_t i;
  snprintf(path, sizeof path, "/proc/%d/cmdline", (int)pid);
  f = fopen(path, "rb");
  if (!f) return 0;
  n = fread(buf, 1, sizeof buf - 1, f);
  fclose(f);
  for (i = 0; i < n; i++) {
    if (buf[i] == '\0') buf[i] = ' ';
  }
  buf[n] = 0;
  return strstr(buf, "WebKitWebProcess") != NULL;
}

static void collect_children(pid_t pid, pid_t *out, int *count, int cap) {
  char path[80];
  FILE *f;
  int child;
  snprintf(path, sizeof path, "/proc/%d/task/%d/children", (int)pid, (int)pid);
  f = fopen(path, "r");
  if (!f) return;
  while (fscanf(f, "%d", &child) == 1) {
    if (*count < cap) out[(*count)++] = (pid_t)child;
    collect_children((pid_t)child, out, count, cap);
  }
  fclose(f);
}

static void kill_web_process(void) {
  pid_t kids[128];
  int count = 0;
  int i;
  pid_t victim = 0;
  collect_children(g_parent.helper, kids, &count, 128);
  printf("WEBPROCESS descendants=%d\n", count);
  for (i = 0; i < count; i++) {
    char path[64];
    char buf[512];
    FILE *f;
    size_t n;
    size_t k;
    snprintf(path, sizeof path, "/proc/%d/cmdline", (int)kids[i]);
    f = fopen(path, "rb");
    if (!f) continue;
    n = fread(buf, 1, sizeof buf - 1, f);
    fclose(f);
    for (k = 0; k < n; k++) {
      if (buf[k] == '\0') buf[k] = ' ';
    }
    buf[n] = 0;
    printf("WEBPROCESS pid=%d cmd=%s\n", (int)kids[i], buf);
    if (descendant_is_web_process(kids[i])) victim = kids[i];
  }
  fflush(stdout);
  if (!victim) {
    say("WEBPROCESS missing");
    return;
  }
  printf("WEBPROCESS kill %d\n", (int)victim);
  fflush(stdout);
  g_parent.web_pid = victim;
  if (kill(victim, SIGKILL) != 0) printf("WEBPROCESS kill_errno=%d\n", errno);
  fflush(stdout);
}

static void finish(void) {
  int adopt = 0;
  int mouse;
  int keyboard;
  int im;
  int entry_non;
  int hidpi;
  int ro;
  int crash;
  int dpr_ok;
  int geom_ok;
  const char *late_plug = "null";

  if (g_parent.socket && gtk_socket_get_plug_window(GTK_SOCKET(g_parent.socket))) {
    late_plug = "non-null";
  }
  adopt = g_parent.plug_id != 0 && g_parent.added > 0 && g_parent.trap == 0 && g_parent.plug_window_live;
  snprintf(g_parent.detail_adopt, sizeof g_parent.detail_adopt,
           "id=%lu added=%d trap=%d plug_window_live=%d late_plug_window=%s loaded=%d",
           g_parent.plug_id, g_parent.added, g_parent.trap, g_parent.plug_window_live, late_plug,
           g_parent.loaded);

  mouse = g_parent.saved_mouse[0] != '\0';
  snprintf(g_parent.detail_mouse, sizeof g_parent.detail_mouse, "mouse=%s xtest=%d",
           g_parent.saved_mouse, g_parent.xtest);
  keyboard = strstr(g_parent.saved_key, "a") != NULL;
  snprintf(g_parent.detail_key, sizeof g_parent.detail_key, "key=%s", g_parent.saved_key);

  im = has_non_ascii(g_parent.saved_text) || has_non_ascii(g_parent.saved_comp);
  snprintf(g_parent.detail_im_web, sizeof g_parent.detail_im_web, "text=%s comp=%s",
           g_parent.saved_text, g_parent.saved_comp);
  entry_non = has_non_ascii(g_parent.entry) || has_non_ascii(g_parent.entry_inserts);
  snprintf(g_parent.detail_im_entry, sizeof g_parent.detail_im_entry,
           "entry=%s inserts=%s non_ascii=%d",
           g_parent.entry, g_parent.entry_inserts, entry_non);

  dpr_ok = strncmp(g_parent.saved_dpr, "2", 1) == 0;
  geom_ok = g_parent.socket_w > 0 && g_parent.plug_w > 0 &&
            (g_parent.plug_w + 4 >= g_parent.socket_w) && (g_parent.socket_w + 4 >= g_parent.plug_w);
  hidpi = g_parent.saved_scale == 2 && dpr_ok && geom_ok;
  snprintf(g_parent.detail_hidpi, sizeof g_parent.detail_hidpi,
           "socket_scale=%d dpr=%s socket_w=%u plug_w=%u",
           g_parent.saved_scale, g_parent.saved_dpr, g_parent.socket_w, g_parent.plug_w);

  ro = strcmp(g_parent.saved_ro_ctor, "ok") == 0 && g_parent.saved_ro_after[0] &&
       g_parent.ro_before[0] && strcmp(g_parent.saved_ro_after, g_parent.ro_before) != 0;
  snprintf(g_parent.detail_ro, sizeof g_parent.detail_ro, "ctor=%s before=%s after=%s",
           g_parent.saved_ro_ctor, g_parent.ro_before, g_parent.saved_ro_after);

  crash = g_parent.saw_crashed && g_parent.alive_at_crash;
  if (g_parent.helper_reaped && WIFEXITED(g_parent.helper_status) &&
      WEXITSTATUS(g_parent.helper_status) == 0) {
    crash = 0;
  }
  snprintf(g_parent.detail_crash, sizeof g_parent.detail_crash,
           "crashed=%d terminated=%d alive=%d reaped=%d status=%d",
           g_parent.saw_crashed, g_parent.saw_terminated, g_parent.alive_at_crash,
           g_parent.helper_reaped, g_parent.helper_status);

  g_parent.result_code = (adopt && mouse && keyboard && im && hidpi && ro && crash) ? 0 : 1;
  printf("RESULT adopt=%s %s\n", adopt ? "pass" : "fail", g_parent.detail_adopt);
  printf("RESULT mouse=%s %s\n", mouse ? "pass" : "fail", g_parent.detail_mouse);
  printf("RESULT keyboard=%s %s\n", keyboard ? "pass" : "fail", g_parent.detail_key);
  printf("RESULT fcitx_xim_entry=%s %s\n", entry_non ? "pass" : "fail",
         g_parent.detail_im_entry);
  printf("RESULT fcitx_xim=%s %s\n", im ? "pass" : "fail", g_parent.detail_im_web);
  printf("RESULT hidpi=%s %s\n", hidpi ? "pass" : "fail", g_parent.detail_hidpi);
  printf("RESULT resize_observer=%s %s\n", ro ? "pass" : "fail", g_parent.detail_ro);
  printf("RESULT web_process_crashed=%s %s\n", crash ? "pass" : "fail", g_parent.detail_crash);
  printf("RESULT checklist=%s\n", g_parent.result_code == 0 ? "pass" : "fail");
  fflush(stdout);
  if (g_parent.helper > 0 && !g_parent.helper_reaped) kill(g_parent.helper, SIGTERM);
  gtk_main_quit();
}

static void request_snap(void) {
  char line[32];
  g_parent.snap++;
  g_parent.js[0] = 0;
  snprintf(line, sizeof line, "snap %d", g_parent.snap);
  parent_send(line);
}

static int js_is_current(void) {
  char needle[32];
  snprintf(needle, sizeof needle, "snap=%d ", g_parent.snap);
  return strncmp(g_parent.js, needle, strlen(needle)) == 0;
}

static void set_phase(int phase) {
  g_parent.phase = phase;
  g_parent.phase_at = g_get_monotonic_time();
}

static void on_added(GtkSocket *socket, gpointer data) {
  (void)data;
  g_parent.added++;
  g_parent.plug_window_live = gtk_socket_get_plug_window(socket) != NULL;
  printf("plug-added count=%d plug_window=%s\n", g_parent.added,
         g_parent.plug_window_live ? "non-null" : "null");
  fflush(stdout);
}

static void adopt(void) {
  GdkDisplay *display = gdk_display_get_default();
  g_parent.window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  gtk_window_set_default_size(GTK_WINDOW(g_parent.window), 720, 560);
  g_parent.socket = gtk_socket_new();
  gtk_container_add(GTK_CONTAINER(g_parent.window), g_parent.socket);
  g_signal_connect(g_parent.socket, "plug-added", G_CALLBACK(on_added), NULL);
  gtk_widget_show_all(g_parent.window);
  gdk_x11_display_error_trap_push(display);
  gtk_socket_add_id(GTK_SOCKET(g_parent.socket), (Window)g_parent.plug_id);
  gdk_display_flush(display);
  g_parent.trap = gdk_x11_display_error_trap_pop(display);
  printf("add_id trap=%d\n", g_parent.trap);
  fflush(stdout);
}

static gboolean tick(gpointer data) {
  gint64 now = g_get_monotonic_time();
  gint64 waited = now - g_parent.phase_at;
  Window win;
  (void)data;

  if (waited > 40 * G_USEC_PER_SEC && g_parent.phase != 90) {
    printf("PHASE_TIMEOUT phase=%d\n", g_parent.phase);
    fflush(stdout);
    finish();
    return FALSE;
  }

  switch (g_parent.phase) {
  case 0:
    if (g_parent.plug_id) {
      adopt();
      set_phase(1);
    }
    break;
  case 1:
    if (g_parent.loaded && waited > 300000) {
      parent_send("scale");
      request_snap();
      set_phase(2);
    }
    break;
  case 2:
    if (js_is_current()) {
      snprintf(g_parent.ro_before, sizeof g_parent.ro_before, "%s", js_field("ro"));
      snprintf(g_parent.saved_dpr, sizeof g_parent.saved_dpr, "%s", js_field("dpr"));
      snprintf(g_parent.saved_ro_ctor, sizeof g_parent.saved_ro_ctor, "%s", js_field("roCtor"));
      log_geometry("initial");
      win = socket_xid();
      if (win && g_parent.xtest) {
        unsigned int w = 0, h = 0, bw = 0, depth = 0;
        int x = 0, y = 0;
        Window root = 0;
        XGetGeometry(g_parent.dpy, win, &root, &x, &y, &w, &h, &bw, &depth);
        click_window(win, (int)w / 2, (int)(h * 3 / 4));
      }
      g_usleep(200000);
      request_snap();
      set_phase(3);
    }
    break;
  case 3:
    if (js_is_current()) {
      snprintf(g_parent.saved_mouse, sizeof g_parent.saved_mouse, "%s", js_field("mouse"));
      fcitx_cmd("fcitx-remote -c", "off");
      tap_key(XK_a);
      g_usleep(150000);
      request_snap();
      set_phase(4);
    }
    break;
  case 4:
    if (js_is_current()) {
      snprintf(g_parent.saved_key, sizeof g_parent.saved_key, "%s", js_field("key"));
      if (!g_parent.ro_before[0]) {
        snprintf(g_parent.ro_before, sizeof g_parent.ro_before, "%s", js_field("ro"));
      }
      g_parent.alloc_before_resize = socket_width();
      gtk_window_resize(GTK_WINDOW(g_parent.window), 480, 360);
      set_phase(41);
    }
    break;
  case 41: {
    unsigned int now_w = socket_width();
    int shrunk = now_w > 0 && g_parent.alloc_before_resize > 4 &&
                 now_w + 4 < g_parent.alloc_before_resize;
    if (shrunk || waited > 2000000) {
      printf("RESIZE before=%u after=%u waited_us=%lld\n",
             g_parent.alloc_before_resize, now_w, (long long)waited);
      fflush(stdout);
      log_geometry("resized");
      set_phase(42);
    }
    break;
  }
  case 42:
    if (waited > 400000) {
      request_snap();
      set_phase(5);
    }
    break;
  case 5:
    if (js_is_current()) {
      char ro_now[64];
      snprintf(ro_now, sizeof ro_now, "%s", js_field("ro"));
      if (g_parent.ro_before[0] && strcmp(ro_now, g_parent.ro_before) == 0 &&
          g_parent.ro_resnap < 3 && waited < 2500000) {
        if (waited < (gint64)(g_parent.ro_resnap + 1) * 600000) break;
        g_parent.ro_resnap++;
        request_snap();
        break;
      }
      snprintf(g_parent.saved_ro_after, sizeof g_parent.saved_ro_after, "%s", ro_now);
      parent_send("focus-entry");
      win = socket_xid();
      if (win && g_parent.xtest) click_window(win, 24, 8);
      fcitx_cmd("fcitx-remote -o", "on");
      fcitx_cmd("fcitx-remote -s pinyin", "pinyin");
      fcitx_cmd("fcitx-remote", "state");
      fcitx_cmd("fcitx-remote -n", "im");
      set_phase(51);
    }
    break;
  case 51:
    if (waited > 200000) {
      type_letters();
      set_phase(52);
    }
    break;
  case 52:
    if (waited > 500000) {
      tap_key(XK_space);
      set_phase(53);
    }
    break;
  case 53:
    if (waited > 400000) {
      tap_key(XK_Return);
      set_phase(54);
    }
    break;
  case 54:
    if (waited > 800000) {
      g_parent.entry[0] = 0;
      g_parent.entry_fresh = 0;
      parent_send("entry");
      set_phase(55);
    }
    break;
  case 55:
    if (g_parent.entry_fresh || waited > 1500000) {
      parent_send("focus-web");
      win = socket_xid();
      if (win && g_parent.xtest) {
        unsigned int w = 0, h = 0, bw = 0, depth = 0;
        int x = 0, y = 0;
        Window root = 0;
        XGetGeometry(g_parent.dpy, win, &root, &x, &y, &w, &h, &bw, &depth);
        click_window(win, (int)w / 2, (int)(h * 3 / 4));
      }
      set_phase(61);
    }
    break;
  case 61:
    if (waited > 200000) {
      type_letters();
      set_phase(62);
    }
    break;
  case 62:
    if (waited > 500000) {
      tap_key(XK_space);
      set_phase(63);
    }
    break;
  case 63:
    if (waited > 400000) {
      tap_key(XK_Return);
      set_phase(64);
    }
    break;
  case 64:
    if (waited > 800000) {
      request_snap();
      set_phase(7);
    }
    break;
  case 7:
    if (js_is_current()) {
      snprintf(g_parent.saved_text, sizeof g_parent.saved_text, "%s", js_field("text"));
      snprintf(g_parent.saved_comp, sizeof g_parent.saved_comp, "%s", js_field("comp"));
      kill_web_process();
      set_phase(8);
    }
    break;
  case 8:
    if (waited > 1000000 && g_parent.web_pid > 0) {
      printf("WEBPROCESS alive_after_kill=%d\n", kill(g_parent.web_pid, 0) == 0);
      fflush(stdout);
      g_parent.web_pid = -1;
    }
    if ((g_parent.saw_crashed && g_parent.helper_reaped) || waited > 8000000) {
      set_phase(90);
      finish();
      return FALSE;
    }
    break;
  default:
    break;
  }
  return TRUE;
}

static void on_helper_line(const char *line) {
  printf("helper: %s\n", line);
  fflush(stdout);
  if (strncmp(line, "PLUG ", 5) == 0) {
    g_parent.plug_id = strtoul(line + 5, NULL, 10);
  } else if (strcmp(line, "LOADED") == 0) {
    g_parent.loaded = 1;
  } else if (strncmp(line, "JS ", 3) == 0) {
    remember_js(line + 3);
  } else if (strncmp(line, "ENTRY_INSERT ", 13) == 0) {
    size_t used = strlen(g_parent.entry_inserts);
    if (used + 1 < sizeof g_parent.entry_inserts) {
      snprintf(g_parent.entry_inserts + used, sizeof g_parent.entry_inserts - used, "%s", line + 13);
    }
  } else if (strncmp(line, "ENTRY ", 6) == 0) {
    snprintf(g_parent.entry, sizeof g_parent.entry, "%s", line + 6);
    g_parent.entry_fresh = 1;
  } else if (strncmp(line, "SIGNAL web-process-crashed", 26) == 0) {
    g_parent.saw_crashed = 1;
    g_parent.alive_at_crash = kill(g_parent.helper, 0) == 0;
  } else if (strncmp(line, "SIGNAL web-process-terminated", 29) == 0) {
    g_parent.saw_terminated = 1;
    if (!g_parent.alive_at_crash) g_parent.alive_at_crash = kill(g_parent.helper, 0) == 0;
  }
}

static gboolean on_helper_stdout(GIOChannel *channel, GIOCondition cond, gpointer data) {
  (void)data;
  if (cond & (G_IO_HUP | G_IO_ERR)) return FALSE;
  for (;;) {
    char *line = NULL;
    GError *err = NULL;
    GIOStatus status = g_io_channel_read_line(channel, &line, NULL, NULL, &err);
    if (status == G_IO_STATUS_NORMAL && line) {
      g_strchomp(line);
      on_helper_line(line);
      g_free(line);
      continue;
    }
    if (err) g_error_free(err);
    if (status == G_IO_STATUS_EOF) return FALSE;
    return TRUE;
  }
}

static void on_child(GPid pid, gint status, gpointer data) {
  (void)pid;
  (void)data;
  g_parent.helper_status = status;
  g_parent.helper_reaped = 1;
  printf("HELPER_EXIT status=%d\n", status);
  fflush(stdout);
  g_spawn_close_pid(pid);
}

static int parent_main(const char *self_path) {
  int stdout_fd = -1;
  GIOChannel *channel;
  GError *err = NULL;
  char *argv[] = {(char *)self_path, "helper", NULL};
  int event_base = 0;
  int error_base = 0;
  int major = 0;
  int minor = 0;

  if (!gtk_init_check(NULL, NULL)) {
    say("gtk_init_check FAILED");
    return 2;
  }
  if (!webkit_is_2281()) {
    say("REFUSE webkit is not 2.28.1");
    return 2;
  }
  print_mapped_webkit();
  printf("display_type=%s\n", G_OBJECT_TYPE_NAME(gdk_display_get_default()));
  fflush(stdout);
  g_parent.dpy = gdk_x11_display_get_xdisplay(gdk_display_get_default());
  g_parent.xtest = XTestQueryExtension(g_parent.dpy, &event_base, &error_base, &major, &minor);
  printf("xtest=%d\n", g_parent.xtest);
  fflush(stdout);
  g_parent.helper_in = -1;
  if (!g_spawn_async_with_pipes(NULL, argv, NULL, G_SPAWN_DO_NOT_REAP_CHILD, NULL, NULL,
                                &g_parent.helper, &g_parent.helper_in, &stdout_fd, NULL, &err)) {
    printf("SPAWN_FAIL %s\n", err && err->message ? err->message : "?");
    fflush(stdout);
    return 2;
  }
  g_child_watch_add(g_parent.helper, on_child, NULL);
  channel = g_io_channel_unix_new(stdout_fd);
  g_io_channel_set_encoding(channel, NULL, NULL);
  g_io_channel_set_flags(channel, G_IO_FLAG_NONBLOCK, NULL);
  g_io_add_watch(channel, G_IO_IN | G_IO_HUP | G_IO_ERR, on_helper_stdout, NULL);
  set_phase(0);
  g_timeout_add(100, tick, NULL);
  gtk_main();
  return g_parent.result_code;
}

int main(int argc, char **argv) {
  const char *backend = getenv("GDK_BACKEND");
  if (!backend || strcmp(backend, "x11") != 0) {
    say("REFUSE GDK_BACKEND must be x11 before gtk_init");
    return 2;
  }
  if (argc > 1 && strcmp(argv[1], "helper") == 0) return helper_main();
  return parent_main(argv[0]);
}
