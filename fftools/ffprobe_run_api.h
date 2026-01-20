#ifndef FFTOOLS_FFPROBE_RUN_API_H
#define FFTOOLS_FFPROBE_RUN_API_H

typedef struct FFProbeContext FFProbeContext;

FFProbeContext *ffprobe_ctx_create(void);
void ffprobe_ctx_free(FFProbeContext *ctx);
void ffprobe_ctx_request_exit(FFProbeContext *ctx);

int ffprobe_run_with_ctx(FFProbeContext *ctx, int argc, char **argv,
                         int install_signal_handlers, int stdin_interaction);
int ffprobe_run_with_options(int argc, char **argv, int install_signal_handlers,
                             int stdin_interaction);

#endif
