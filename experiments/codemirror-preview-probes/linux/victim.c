#include <gtk/gtk.h>
#include <stdio.h>

int main(int argc, char **argv) {
  printf("victim_start\n");
  fflush(stdout);
  if (!gtk_init_check(&argc, &argv)) {
    printf("victim_gtk_init_check FAILED\n");
    return 3;
  }
  printf("victim_gtk_ok type=%s\n", G_OBJECT_TYPE_NAME(gdk_display_get_default()));
  return 0;
}
