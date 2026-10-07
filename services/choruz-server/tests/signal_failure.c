#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>

#ifdef __APPLE__
static int deny_sigint(int signal, const struct sigaction *action, struct sigaction *old) {
    if (signal == SIGINT) { errno = EINVAL; return -1; }
    return sigaction(signal, action, old);
}
__attribute__((used)) static struct { const void *replacement; const void *original; }
interpose __attribute__((section("__DATA,__interpose"))) = { (void *)deny_sigint, (void *)sigaction };
#else
#include <dlfcn.h>
int sigaction(int signal, const struct sigaction *action, struct sigaction *old) {
    if (signal == SIGINT) { errno = EINVAL; return -1; }
    int (*original)(int, const struct sigaction *, struct sigaction *) = dlsym(RTLD_NEXT, "sigaction");
    return original(signal, action, old);
}
#endif
