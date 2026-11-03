#include <stdio.h>

__attribute__((constructor)) static void loaded(void) {
  fprintf(stderr, "STUB_LIBGTK_LOADED\n");
}
