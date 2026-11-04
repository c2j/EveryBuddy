/* Opt-in Linux preview helper. Not used unless EB_PREVIEW=webview.
   Links GTK only. WebKitGTK is dlopened from the system, and only 2.28
   continues. The preview URL arrives on stdin and is never printed. */
#include <gtk/gtk.h>
#include <gtk/gtkx.h>

#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static GtkWidget *g_plug;
static GtkWidget *g_web;
static char g_url[2048];

typedef unsigned (*VersionFn)(void);
typedef GtkWidget *(*NewViewFn)(void);
typedef void (*LoadUriFn)(GtkWidget *, const char *);
typedef void (*RunJsFn)(void *, const char *, void *, void *, void *);

static void *g_webkit;
static LoadUriFn g_load_uri;
static RunJsFn g_run_js;

static void say(const char *text) {
  fputs(text, stdout);
  fputc('\n', stdout);
  fflush(stdout);
}

static int read_line(int fd, char *buf, size_t cap) {
  size_t used = 0;
  if (cap == 0) return -1;
  while (used + 1 < cap) {
    char byte = 0;
    ssize_t got = read(fd, &byte, 1);
    if (got <= 0) return -1;
    if (byte == '\n') break;
    buf[used++] = byte;
  }
  buf[used] = 0;
  return 0;
}

static int webkit_is_228(void) {
  VersionFn major = (VersionFn)dlsym(g_webkit, "webkit_get_major_version");
  VersionFn minor = (VersionFn)dlsym(g_webkit, "webkit_get_minor_version");
  VersionFn micro = (VersionFn)dlsym(g_webkit, "webkit_get_micro_version");
  unsigned maj;
  unsigned min;
  unsigned mic;
  if (!major || !minor || !micro) {
    say("REFUSE webkit-missing");
    return 0;
  }
  maj = major();
  min = minor();
  mic = micro();
  if (maj == 2 && min == 28) return 1;
  printf("REFUSE webkit %u.%u.%u\n", maj, min, mic);
  fflush(stdout);
  return 0;
}

static int load_webkit(void) {
  NewViewFn new_view;
  g_webkit = dlopen("libwebkit2gtk-4.0.so.37", RTLD_NOW | RTLD_LOCAL);
  if (!g_webkit) g_webkit = dlopen("libwebkit2gtk-4.0.so.0", RTLD_NOW | RTLD_LOCAL);
  if (!g_webkit) return 0;
  if (!webkit_is_228()) return 0;
  new_view = (NewViewFn)dlsym(g_webkit, "webkit_web_view_new");
  g_load_uri = (LoadUriFn)dlsym(g_webkit, "webkit_web_view_load_uri");
  g_run_js = (RunJsFn)dlsym(g_webkit, "webkit_web_view_run_javascript");
  if (!new_view || !g_load_uri) {
    say("REFUSE webkit-missing");
    return 0;
  }
  g_web = new_view();
  if (!g_web) say("REFUSE webkit-missing");
  return g_web != NULL;
}

static gboolean exit_crashed(gpointer data) {
  (void)data;
  exit(3);
  return FALSE;
}

static gboolean on_crashed(GtkWidget *web, gpointer data) {
  (void)web;
  (void)data;
  say("CRASHED");
  g_idle_add(exit_crashed, NULL);
  return TRUE;
}

static void wake_page(void) {
  if (!g_web) return;
  if (g_run_js) {
    g_run_js(g_web, "if (window.__ebPreviewWake) window.__ebPreviewWake()", NULL, NULL, NULL);
    return;
  }
  if (g_load_uri && g_url[0]) g_load_uri(g_web, g_url);
}

static gboolean on_stdin(GIOChannel *channel, GIOCondition cond, gpointer data) {
  (void)data;
  if (cond & (G_IO_HUP | G_IO_ERR)) return FALSE;
  for (;;) {
    char *line = NULL;
    GError *err = NULL;
    GIOStatus status = g_io_channel_read_line(channel, &line, NULL, NULL, &err);
    if (status == G_IO_STATUS_NORMAL && line) {
      g_strchomp(line);
      if (strcmp(line, "WAKE") == 0) wake_page();
      g_free(line);
      continue;
    }
    if (err) g_error_free(err);
    return status != G_IO_STATUS_EOF;
  }
}

int main(void) {
  const char *backend;
  unsigned long plug_id;
  GIOChannel *channel;

  if (read_line(STDIN_FILENO, g_url, sizeof g_url) != 0 || strncmp(g_url, "PLUG_URL ", 9) != 0) {
    say("REFUSE no-url");
    return 2;
  }
  memmove(g_url, g_url + 9, strlen(g_url + 9) + 1);

  backend = getenv("GDK_BACKEND");
  if (!backend || strcmp(backend, "x11") != 0) {
    say("REFUSE gtk_init");
    return 2;
  }
  if (!gtk_init_check(NULL, NULL)) {
    say("REFUSE gtk_init");
    return 2;
  }
  if (!load_webkit()) {
    if (!g_webkit) say("REFUSE webkit-missing");
    return 2;
  }

  g_plug = gtk_plug_new(0);
  gtk_container_add(GTK_CONTAINER(g_plug), g_web);
  g_signal_connect(g_web, "web-process-crashed", G_CALLBACK(on_crashed), NULL);
  gtk_widget_show_all(g_plug);
  plug_id = (unsigned long)gtk_plug_get_id(GTK_PLUG(g_plug));
  if (plug_id == 0) {
    say("REFUSE plug-id");
    return 2;
  }
  g_load_uri(g_web, g_url);
  printf("PLUG %lu\n", plug_id);
  fflush(stdout);

  channel = g_io_channel_unix_new(STDIN_FILENO);
  g_io_channel_set_encoding(channel, NULL, NULL);
  g_io_channel_set_flags(channel, G_IO_FLAG_NONBLOCK, NULL);
  g_io_add_watch(channel, G_IO_IN | G_IO_HUP | G_IO_ERR, on_stdin, NULL);
  gtk_main();
  return 0;
}
