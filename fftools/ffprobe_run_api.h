#ifndef FFTOOLS_FFPROBE_RUN_API_H
#define FFTOOLS_FFPROBE_RUN_API_H

#include <stdint.h>

typedef struct FFProbeContext FFProbeContext;
typedef int (*ffprobe_write_cb)(void *opaque, const uint8_t *buf, int len);

FFProbeContext *ffprobe_ctx_create(void);
void ffprobe_ctx_free(FFProbeContext *ctx);
void ffprobe_ctx_request_exit(FFProbeContext *ctx);
void ffprobe_ctx_set_output(FFProbeContext *ctx, ffprobe_write_cb out_cb, void *out_opaque,
                            ffprobe_write_cb err_cb, void *err_opaque);

int ffprobe_run_with_ctx(FFProbeContext *ctx, int argc, char **argv,
                         int install_signal_handlers, int stdin_interaction);
int ffprobe_run_with_options(int argc, char **argv, int install_signal_handlers,
                             int stdin_interaction);

#endif
