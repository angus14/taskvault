#define _GNU_SOURCE
#include <dlfcn.h>
#include <sys/resource.h>
int setrlimit(__rlimit_resource_t resource, const struct rlimit *requested) {
    int (*real_set)(__rlimit_resource_t, const struct rlimit *) = dlsym(RTLD_NEXT, "setrlimit");
    struct rlimit current, bounded = *requested;
    if (resource == RLIMIT_NOFILE && getrlimit(resource, &current) == 0) {
        if (bounded.rlim_max > current.rlim_max) bounded.rlim_max = current.rlim_max;
        if (bounded.rlim_cur > bounded.rlim_max) bounded.rlim_cur = bounded.rlim_max;
    }
    return real_set(resource, &bounded);
}
int setrlimit64(__rlimit_resource_t resource, const struct rlimit64 *requested) {
    int (*real_set)(__rlimit_resource_t, const struct rlimit64 *) = dlsym(RTLD_NEXT, "setrlimit64");
    struct rlimit64 current, bounded = *requested;
    if (resource == RLIMIT_NOFILE && getrlimit64(resource, &current) == 0) {
        if (bounded.rlim_max > current.rlim_max) bounded.rlim_max = current.rlim_max;
        if (bounded.rlim_cur > bounded.rlim_max) bounded.rlim_cur = bounded.rlim_max;
    }
    return real_set(resource, &bounded);
}
