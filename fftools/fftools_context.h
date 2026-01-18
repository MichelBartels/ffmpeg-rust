#ifndef FFTOOLS_CONTEXT_H
#define FFTOOLS_CONTEXT_H

#include "config.h"

#include <stdint.h>
#include <stdatomic.h>
#include <stdio.h>
#include <signal.h>
#ifdef HAVE_TERMIOS_H
#include <termios.h>
#endif

#include "libavformat/avio.h"
#include "libavutil/dict.h"

typedef struct InputFile InputFile;
typedef struct OutputFile OutputFile;
typedef struct FilterGraph FilterGraph;
typedef struct HWDevice HWDevice;
typedef struct Decoder Decoder;

typedef struct BenchmarkTimeStamps {
    int64_t real_usec;
    int64_t user_usec;
    int64_t sys_usec;
} BenchmarkTimeStamps;


typedef struct FftoolsContext {
    /* cmdutils.c */
    AVDictionary *sws_dict;
    AVDictionary *swr_opts;
    AVDictionary *format_opts;
    AVDictionary *codec_opts;
    int hide_banner;

    /* ffmpeg_opt.c */
    HWDevice *filter_hw_device;
    char *vstats_filename;
    float dts_delta_threshold;
    float dts_error_threshold;
    float frame_drop_threshold;
    int do_benchmark;
    int do_benchmark_all;
    int do_hex_dump;
    int do_pkt_dump;
    int copy_ts;
    int start_at_zero;
    int copy_tb;
    int debug_ts;
    int exit_on_error;
    int abort_on_flags;
    int print_stats;
    int stdin_interaction;
    float max_error_rate;
    char *filter_nbthreads;
    int filter_complex_nbthreads;
    int filter_buffered_frames;
    int vstats_version;
    int print_graphs;
    char *print_graphs_file;
    char *print_graphs_format;
    int auto_conversion_filters;
    int ignore_unknown_streams;
    int copy_unknown_streams;
    int recast_media;
    int64_t stats_period;

    /* run-local counters/state (ffmpeg.c) */
    atomic_uint nb_output_dumped;
    BenchmarkTimeStamps current_time;
    atomic_int transcode_init_done;
    volatile sig_atomic_t received_sigterm;
    volatile sig_atomic_t received_nb_signals;
    int install_signal_handlers;
#ifdef HAVE_TERMIOS_H
    int restore_tty;
    struct termios oldtty;
#endif

    /* logging/reporting (opt_common.c) */
    FILE *report_file;
    int report_file_level;
    int warned_cfg;

    /* ffmpeg.c/ffmpeg_* core state */
    FILE *vstats_file;
    AVIOContext *progress_avio;
    InputFile **input_files;
    int nb_input_files;
    OutputFile **output_files;
    int nb_output_files;
    FilterGraph **filtergraphs;
    int nb_filtergraphs;
    Decoder **decoders;
    int nb_decoders;
} FftoolsContext;

extern FftoolsContext fftools_global_ctx;
extern _Thread_local FftoolsContext *fftools_ctx;

FftoolsContext *fftools_default_context(void);
FftoolsContext *fftools_set_context(FftoolsContext *ctx);

int ffmpeg_run(FftoolsContext *ctx, int argc, char **argv);

#endif
