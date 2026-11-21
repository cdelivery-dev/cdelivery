#define _GNU_SOURCE

#include "secret-env.h"

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int inject_secret_env(void) {
    DIR *directory = opendir("/run/cdelivery/env");
    if (directory == NULL) {
        if (errno == ENOENT) return 0;
        fprintf(stderr, "[FATAL tini] open secret directory: %s\n", strerror(errno));
        return 1;
    }
    struct dirent *entry;
    int failure = 0;
    errno = 0;
    while ((entry = readdir(directory)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) {
            errno = 0;
            continue;
        }
        int fd = openat(dirfd(directory), entry->d_name, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
        if (fd < 0) { failure = errno; break; }
        struct stat info;
        if (fstat(fd, &info) != 0) {
            failure = errno;
            close(fd);
            break;
        }
        if (!S_ISREG(info.st_mode) || info.st_size < 0 || info.st_size > 511999) {
            failure = EINVAL;
            close(fd);
            break;
        }
        size_t size = (size_t) info.st_size;
        char *value = malloc(size + 1);
        if (value == NULL) {
            failure = ENOMEM;
            close(fd);
            break;
        }
        size_t offset = 0;
        while (offset < size) {
            ssize_t count = read(fd, value + offset, size - offset);
            if (count < 0 && errno == EINTR) continue;
            if (count <= 0) { failure = count < 0 ? errno : EIO; break; }
            offset += (size_t) count;
        }
        close(fd);
        value[size] = '\0';
        char *selected = value;
        char *field = strrchr(entry->d_name, '@');
        if (!failure && field != NULL) {
            *field++ = '\0';
            if (entry->d_name[0] == '\0' || field[0] == '\0') failure = EINVAL;
            selected = NULL;
            size_t field_size = strlen(field);
            for (size_t start = 0; !failure && start < size;) {
                char *end = memchr(value + start, '\0', size - start);
                if (end == NULL) { failure = EINVAL; break; }
                size_t record_size = (size_t) (end - value - start);
                char *equals = memchr(value + start, '=', record_size);
                if (equals == NULL || equals == value + start) { failure = EINVAL; break; }
                if ((size_t) (equals - value - start) == field_size &&
                    memcmp(value + start, field, field_size) == 0) {
                    if (selected != NULL) { failure = EINVAL; break; }
                    selected = equals + 1;
                }
                start += record_size + 1;
            }
            if (selected == NULL) failure = EINVAL;
        }
        if (!failure && setenv(entry->d_name, selected, 1) != 0) failure = errno;
        free(value);
        if (failure) break;
        errno = 0;
    }
    if (!failure) failure = errno;
    closedir(directory);
    if (failure) {
        fprintf(stderr, "[FATAL tini] inject secret environment: %s\n", strerror(failure));
        return 1;
    }
    return 0;
}
