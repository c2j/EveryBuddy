#include <gtk/gtk.h>
#include <stdio.h>

static const char *env_of(const char *name) {
  const char *value = g_getenv(name);
  return value ? value : "(unset)";
}

int main(int argc, char **argv) {
  printf("before_init GDK_BACKEND=%s\n", env_of("GDK_BACKEND"));
  if (!gtk_init_check(&argc, &argv)) {
    printf("gtk_init_check FAILED\n");
    return 2;
  }
  GdkDisplay *display = gdk_display_get_default();
  printf("after_init type=%s name=%s GDK_BACKEND=%s\n",
         G_OBJECT_TYPE_NAME(display),
         gdk_display_get_name(display),
         env_of("GDK_BACKEND"));
  g_setenv("GDK_BACKEND", "x11", TRUE);
  display = gdk_display_get_default();
  printf("after_setenv type=%s name=%s GDK_BACKEND=%s\n",
         G_OBJECT_TYPE_NAME(display),
         gdk_display_get_name(display),
         env_of("GDK_BACKEND"));
  return 0;
}
