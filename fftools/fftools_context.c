#include "fftools/fftools_context.h"

#include "libavutil/avutil.h"

FftoolsContext fftools_global_ctx = {
    .sws_dict = NULL,
    .swr_opts = NULL,
    .format_opts = NULL,
    .codec_opts = NULL,
    .hide_banner = 0,

    .filter_hw_device = NULL,
    .vstats_filename = NULL,
    .dts_delta_threshold = 10,
    .dts_error_threshold = 3600 * 30,
    .frame_drop_threshold = 0,
    .do_benchmark = 0,
    .do_benchmark_all = 0,
    .do_hex_dump = 0,
    .do_pkt_dump = 0,
    .copy_ts = 0,
    .start_at_zero = 0,
    .copy_tb = -1,
    .debug_ts = 0,
    .exit_on_error = 0,
    .abort_on_flags = 0,
    .print_stats = -1,
    .stdin_interaction = 1,
    .max_error_rate = 2.0 / 3,
    .filter_nbthreads = NULL,
    .filter_complex_nbthreads = 0,
    .filter_buffered_frames = 0,
    .vstats_version = 2,
    .print_graphs = 0,
    .print_graphs_file = NULL,
    .print_graphs_format = NULL,
    .auto_conversion_filters = 1,
    .ignore_unknown_streams = 0,
    .copy_unknown_streams = 0,
    .recast_media = 0,
    .stats_period = 500000,

    .nb_output_dumped = 0,
    .current_time = {0, 0, 0},
    .transcode_init_done = 0,
    .received_sigterm = 0,
    .received_nb_signals = 0,
    .install_signal_handlers = 1,
#ifdef HAVE_TERMIOS_H
    .restore_tty = 0,
    .oldtty = {0},
#endif

    .report_file = NULL,
    .report_file_level = AV_LOG_DEBUG,
    .warned_cfg = 0,

    .vstats_file = NULL,
    .progress_avio = NULL,
    .input_files = NULL,
    .nb_input_files = 0,
    .output_files = NULL,
    .nb_output_files = 0,
    .filtergraphs = NULL,
    .nb_filtergraphs = 0,
    .decoders = NULL,
    .nb_decoders = 0,
};

_Thread_local FftoolsContext *fftools_ctx = &fftools_global_ctx;

FftoolsContext *fftools_default_context(void)
{
    return &fftools_global_ctx;
}

FftoolsContext *fftools_set_context(FftoolsContext *ctx)
{
    FftoolsContext *prev = fftools_ctx;
    fftools_ctx = ctx ? ctx : &fftools_global_ctx;
    return prev;
}

