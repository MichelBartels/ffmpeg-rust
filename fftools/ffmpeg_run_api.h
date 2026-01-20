#ifndef FFTOOLS_FFMPEG_RUN_API_H
#define FFTOOLS_FFMPEG_RUN_API_H

typedef struct FftoolsContext FftoolsContext;

FftoolsContext *ffmpeg_ctx_create(int install_signal_handlers,
                                  int stdin_interaction);
void ffmpeg_ctx_free(FftoolsContext *ctx);
void ffmpeg_ctx_request_exit(FftoolsContext *ctx);

int ffmpeg_run_with_ctx(FftoolsContext *ctx, int argc, char **argv);
int ffmpeg_run_with_options(int argc, char **argv, int install_signal_handlers,
                            int stdin_interaction);

#endif
