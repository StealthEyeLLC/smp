#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int nibble(char value) {
    if (value >= '0' && value <= '9') return value - '0';
    if (value >= 'a' && value <= 'f') return value - 'a' + 10;
    if (value >= 'A' && value <= 'F') return value - 'A' + 10;
    return -1;
}

static char *decode_path(const char *input) {
    size_t length = strlen(input);
    if (length < 2 || (length % 2) != 0) return NULL;
    char *output = calloc(length / 2 + 1, 1);
    if (!output) return NULL;
    for (size_t index = 0; index < length; index += 2) {
        int high = nibble(input[index]);
        int low = nibble(input[index + 1]);
        if (high < 0 || low < 0) { free(output); return NULL; }
        output[index / 2] = (char)((high << 4) | low);
        if (output[index / 2] == '\0') { free(output); return NULL; }
    }
    if (output[0] != '/' || strstr(output, "/../") || strstr(output, "/./") || strcmp(output, "/..") == 0) {
        free(output);
        return NULL;
    }
    return output;
}

static int copy_exact(int input, int output, uint64_t maximum) {
    unsigned char buffer[65536];
    uint64_t total = 0;
    while (total < maximum) {
        size_t wanted = sizeof(buffer);
        if (maximum - total < wanted) wanted = (size_t)(maximum - total);
        ssize_t count = read(input, buffer, wanted);
        if (count < 0) { if (errno == EINTR) continue; return -1; }
        if (count == 0) break;
        size_t written = 0;
        while (written < (size_t)count) {
            ssize_t value = write(output, buffer + written, (size_t)count - written);
            if (value < 0) { if (errno == EINTR) continue; return -1; }
            written += (size_t)value;
        }
        total += (uint64_t)count;
    }
    return total == maximum ? 0 : 1;
}

int main(int argc, char **argv) {
    const char *program = strrchr(argv[0], '/');
    program = program ? program + 1 : argv[0];
    if (strcmp(program, "smp-file-write-hex") == 0) {
        if (argc != 3) return 64;
        char *path = decode_path(argv[1]);
        if (!path) return 65;
        char *end = NULL;
        uint64_t length = strtoull(argv[2], &end, 10);
        if (!end || *end != '\0') return 65;
        int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC | O_NOFOLLOW, 0600);
        if (fd < 0) { perror("open"); return 74; }
        int result = copy_exact(STDIN_FILENO, fd, length);
        if (fsync(fd) != 0) result = -1;
        if (close(fd) != 0) result = -1;
        if (result != 0) { fprintf(stderr, "smp-file-write-hex: input length mismatch or I/O failure\n"); return 74; }
        return 0;
    }
    if (strcmp(program, "smp-file-read-hex") == 0) {
        if (argc != 4) return 64;
        char *path = decode_path(argv[1]);
        if (!path) return 65;
        char *end_offset = NULL;
        char *end_max = NULL;
        uint64_t offset = strtoull(argv[2], &end_offset, 10);
        uint64_t maximum = strtoull(argv[3], &end_max, 10);
        if (!end_offset || *end_offset != '\0' || !end_max || *end_max != '\0') return 65;
        int fd = open(path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
        if (fd < 0) { perror("open"); return 74; }
        if (lseek(fd, (off_t)offset, SEEK_SET) < 0) { perror("lseek"); return 74; }
        unsigned char buffer[65536];
        uint64_t total = 0;
        while (total < maximum) {
            size_t wanted = sizeof(buffer);
            if (maximum - total < wanted) wanted = (size_t)(maximum - total);
            ssize_t count = read(fd, buffer, wanted);
            if (count < 0) { if (errno == EINTR) continue; perror("read"); return 74; }
            if (count == 0) break;
            if (write(STDOUT_FILENO, buffer, (size_t)count) != count) return 74;
            total += (uint64_t)count;
        }
        return 0;
    }
    fprintf(stderr, "smp-file-hex: invoke as smp-file-write-hex or smp-file-read-hex\n");
    return 64;
}
