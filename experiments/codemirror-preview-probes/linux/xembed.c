#include <gtk/gtk.h>
#include <gtk/gtkx.h>
#include <stdio.h>

#ifndef GDK_WINDOWING_X11
#error "this GTK build has no X11 socket"
#endif

static int added = 0;

static void on_added(GtkSocket *socket, gpointer data) {
  (void)socket;
  (void)data;
  added++;
}

static void drain(void) {
  while (gtk_events_pending()) {
    gtk_main_iteration();
  }
}

static void say(const char *text) {
  fputs(text, stdout);
  fputc('\n', stdout);
  fflush(stdout);
}

static GtkWidget *host_with_socket(void) {
  GtkWidget *window = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  GtkWidget *socket = gtk_socket_new();
  gtk_container_add(GTK_CONTAINER(window), socket);
  gtk_widget_show_all(window);
  g_signal_connect(socket, "plug-added", G_CALLBACK(on_added), NULL);
  return socket;
}

int main(int argc, char **argv) {
  if (!gtk_init_check(&argc, &argv)) {
    printf("gtk_init_check FAILED\n");
    return 2;
  }
  GdkDisplay *display = gdk_display_get_default();
  printf("display_type=%s\n", G_OBJECT_TYPE_NAME(display));
  fflush(stdout);

  GtkWidget *plug = gtk_plug_new(0);
  printf("plug_id_before_realize=%lu\n", (unsigned long)gtk_plug_get_id(GTK_PLUG(plug)));
  fflush(stdout);
  gtk_widget_realize(plug);
  Window plug_id = gtk_plug_get_id(GTK_PLUG(plug));
  printf("plug_id_after_realize=%lu\n", (unsigned long)plug_id);
  fflush(stdout);

  GtkWidget *plain = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  gtk_widget_realize(plain);
  Window plain_xid = gdk_x11_window_get_xid(gtk_widget_get_window(plain));
  printf("plain_toplevel_xid=%lu\n", (unsigned long)plain_xid);
  printf("plain_xid_equals_plug_id=%d\n", plain_xid == plug_id);
  fflush(stdout);

  added = 0;
  GtkWidget *plug_socket = host_with_socket();
  gtk_widget_show(plug);
  gtk_socket_add_id(GTK_SOCKET(plug_socket), plug_id);
  drain();
  printf("add_id_plug_id plug_window=%s added=%d\n",
         gtk_socket_get_plug_window(GTK_SOCKET(plug_socket)) ? "non-null" : "null",
         added);
  fflush(stdout);

  added = 0;
  GtkWidget *zero_socket = host_with_socket();
  gtk_socket_add_id(GTK_SOCKET(zero_socket), 0);
  drain();
  printf("add_id_0 plug_window=%s added=%d\n",
         gtk_socket_get_plug_window(GTK_SOCKET(zero_socket)) ? "non-null" : "null",
         added);
  fflush(stdout);

  say("add_id_plain_xid begin");
  added = 0;
  GtkWidget *plain_socket = host_with_socket();
  gtk_socket_add_id(GTK_SOCKET(plain_socket), plain_xid);
  drain();
  printf("add_id_plain_xid plug_window=%s added=%d\n",
         gtk_socket_get_plug_window(GTK_SOCKET(plain_socket)) ? "non-null" : "null",
         added);
  fflush(stdout);
  return 0;
}
