#include "fftools/fftools_context.h"
#include "fftools/ffmpeg_run_api.h"

int ffmpeg_run_with_options(int argc, char **argv, int install_signal_handlers,
                            int stdin_interaction)
{
    FftoolsContext ctx = *fftools_default_context();

    ctx.install_signal_handlers = install_signal_handlers;
    ctx.stdin_interaction = stdin_interaction;

    return ffmpeg_run(&ctx, argc, argv);
}
