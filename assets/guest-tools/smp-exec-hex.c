#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int nibble(char value) {
    if (value >= '0' && value <= '9') return value - '0';
    if (value >= 'a' && value <= 'f') return value - 'a' + 10;
    if (value >= 'A' && value <= 'F') return value - 'A' + 10;
    return -1;
}

static char *decode(const char *input) {
    size_t length = strlen(input);
    if (length == 0 || (length % 2) != 0) return NULL;
    char *output = calloc(length / 2 + 1, 1);
    if (!output) return NULL;
    for (size_t index = 0; index < length; index += 2) {
        int high = nibble(input[index]);
        int low = nibble(input[index + 1]);
        if (high < 0 || low < 0) { free(output); return NULL; }
        output[index / 2] = (char)((high << 4) | low);
        if (output[index / 2] == '\0') { free(output); return NULL; }
    }
    return output;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "smp-exec-hex: expected at least one encoded argv element\n");
        return 64;
    }
    char **decoded = calloc((size_t)argc, sizeof(char *));
    if (!decoded) return 70;
    for (int index = 1; index < argc; index++) {
        decoded[index - 1] = decode(argv[index]);
        if (!decoded[index - 1]) {
            fprintf(stderr, "smp-exec-hex: invalid hex argv at index %d\n", index - 1);
            return 65;
        }
    }
    decoded[argc - 1] = NULL;
    execvp(decoded[0], decoded);
    fprintf(stderr, "smp-exec-hex: execvp: %s\n", strerror(errno));
    return errno == ENOENT ? 127 : 126;
}
