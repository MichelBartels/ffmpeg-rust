#include "fftools/fftools_context.h"
#include "fftools/ffmpeg_run_api.h"

#include <signal.h>
#include <stdlib.h>
#include "libavutil/mem.h"

FftoolsContext *ffmpeg_ctx_create(int install_signal_handlers,
                                  int stdin_interaction)
{
    FftoolsContext *ctx = av_mallocz(sizeof(*ctx));
    if (!ctx)
        return NULL;
    *ctx = *fftools_default_context();
    ctx->install_signal_handlers = install_signal_handlers;
    ctx->stdin_interaction = stdin_interaction;
    return ctx;
}

void ffmpeg_ctx_free(FftoolsContext *ctx)
{
    if (!ctx)
        return;
    av_free(ctx);
}

void ffmpeg_ctx_request_exit(FftoolsContext *ctx)
{
    if (!ctx)
        return;
    ctx->received_sigterm = SIGINT;
    ctx->received_nb_signals = 2;
}

int ffmpeg_run_with_ctx(FftoolsContext *ctx, int argc, char **argv)
{
    return ffmpeg_run(ctx, argc, argv);
}

int ffmpeg_run_with_options(int argc, char **argv, int install_signal_handlers,
                            int stdin_interaction)
{
    FftoolsContext ctx = *fftools_default_context();
    ctx.install_signal_handlers = install_signal_handlers;
    ctx.stdin_interaction = stdin_interaction;
    return ffmpeg_run(&ctx, argc, argv);
}
