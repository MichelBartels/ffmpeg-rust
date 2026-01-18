#ifndef FFTOOLS_FFPROBE_RUN_API_H
#define FFTOOLS_FFPROBE_RUN_API_H

int ffprobe_run_with_options(int argc, char **argv, int install_signal_handlers,
                             int stdin_interaction);

#endif
