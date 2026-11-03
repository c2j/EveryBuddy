#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main(int argc, char **argv) {
  if (argc < 3) {
    fprintf(stderr, "usage: parent_exec keep|clean victim\n");
    return 2;
  }
  if (argv[1][0] == 'c') {
    unsetenv("LD_LIBRARY_PATH");
  } else {
    setenv("LD_LIBRARY_PATH", "./bundle/lib", 1);
  }
  char *args[2];
  args[0] = argv[2];
  args[1] = NULL;
  execv(argv[2], args);
  perror("execv");
  return 1;
}
