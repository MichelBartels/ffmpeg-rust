#ifndef FFTOOLS_FFMPEG_RUN_API_H
#define FFTOOLS_FFMPEG_RUN_API_H

int ffmpeg_run_with_options(int argc, char **argv, int install_signal_handlers,
                            int stdin_interaction);

#endif
