#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <inttypes.h>

#include <libavformat/avformat.h>
#include <libavcodec/avcodec.h>
#include <libavutil/opt.h>
#include <libavutil/channel_layout.h>
#include <libavutil/timestamp.h>
#include <libavutil/mathematics.h>
#include <libswresample/swresample.h>

static void usage(const char *prog) {
    fprintf(stderr,
            "Usage: %s <input_url> <playlist.m3u8> <segment_time_s> <audio_bitrate_kbps> [max_seconds]\n",
            prog);
}

static char *make_segment_pattern(const char *playlist) {
    const char *slash = strrchr(playlist, '/');
    if (!slash) {
        return av_strdup("./seg_%05d.m4s");
    }
    size_t dir_len = (size_t)(slash - playlist);
    size_t out_len = dir_len + 1 + strlen("seg_%05d.m4s") + 1;
    char *out = av_malloc(out_len);
    if (!out) return NULL;
    memcpy(out, playlist, dir_len);
    out[dir_len] = '/';
    snprintf(out + dir_len + 1, out_len - dir_len - 1, "%s", "seg_%05d.m4s");
    return out;
}

static int64_t rescale_ts(int64_t ts, AVRational src, AVRational dst) {
    if (ts == AV_NOPTS_VALUE) return AV_NOPTS_VALUE;
    return av_rescale_q(ts, src, dst);
}

static int open_audio_decoder(AVStream *st, AVCodecContext **dec_ctx) {
    const AVCodec *dec = avcodec_find_decoder(st->codecpar->codec_id);
    if (!dec) return AVERROR_DECODER_NOT_FOUND;
    *dec_ctx = avcodec_alloc_context3(dec);
    if (!*dec_ctx) return AVERROR(ENOMEM);
    int ret = avcodec_parameters_to_context(*dec_ctx, st->codecpar);
    if (ret < 0) return ret;
    return avcodec_open2(*dec_ctx, dec, NULL);
}

static int open_aac_encoder(AVCodecContext **enc_ctx, int sample_rate, int bitrate_kbps) {
    const AVCodec *enc = avcodec_find_encoder(AV_CODEC_ID_AAC);
    if (!enc) return AVERROR_ENCODER_NOT_FOUND;
    *enc_ctx = avcodec_alloc_context3(enc);
    if (!*enc_ctx) return AVERROR(ENOMEM);

    (*enc_ctx)->bit_rate = (int64_t)bitrate_kbps * 1000;
    (*enc_ctx)->sample_rate = sample_rate > 0 ? sample_rate : 48000;
    (*enc_ctx)->time_base = (AVRational){1, (*enc_ctx)->sample_rate};

    av_channel_layout_default(&(*enc_ctx)->ch_layout, 2);

    // Pick a supported sample format; prefer FLTP if present.
    const enum AVSampleFormat *p = enc->sample_fmts;
    (*enc_ctx)->sample_fmt = p ? p[0] : AV_SAMPLE_FMT_FLTP;
    if (p) {
        for (int i = 0; p[i] != AV_SAMPLE_FMT_NONE; i++) {
            if (p[i] == AV_SAMPLE_FMT_FLTP) {
                (*enc_ctx)->sample_fmt = p[i];
                break;
            }
        }
    }

    return avcodec_open2(*enc_ctx, enc, NULL);
}

int main(int argc, char **argv) {
    if (argc < 5) {
        usage(argv[0]);
        return 1;
    }

    const char *input_url = argv[1];
    const char *playlist = argv[2];
    int segment_time = atoi(argv[3]);
    int audio_bitrate_kbps = atoi(argv[4]);
    int max_seconds = -1;
    if (argc >= 6) {
        max_seconds = atoi(argv[5]);
    }

    int ret = 0;
    AVFormatContext *ifmt = NULL;
    AVFormatContext *ofmt = NULL;
    AVCodecContext *aud_dec_ctx = NULL;
    AVCodecContext *aud_enc_ctx = NULL;
    SwrContext *swr = NULL;
    AVPacket *pkt = NULL;
    AVFrame *frame = NULL;
    AVFrame *resampled = NULL;

    av_log_set_level(AV_LOG_INFO);
    avformat_network_init();

    AVDictionary *in_opts = NULL;
    av_dict_set(&in_opts, "fflags", "+genpts", 0);

    if ((ret = avformat_open_input(&ifmt, input_url, NULL, &in_opts)) < 0) {
        fprintf(stderr, "Failed to open input: %s\n", av_err2str(ret));
        return 1;
    }
    av_dict_free(&in_opts);

    if ((ret = avformat_find_stream_info(ifmt, NULL)) < 0) {
        fprintf(stderr, "Failed to find stream info: %s\n", av_err2str(ret));
        return 1;
    }

    int v_idx = av_find_best_stream(ifmt, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    int a_idx = av_find_best_stream(ifmt, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);

    if (v_idx < 0 && a_idx < 0) {
        fprintf(stderr, "No audio or video streams found.\n");
        return 1;
    }

    if ((ret = avformat_alloc_output_context2(&ofmt, NULL, "hls", playlist)) < 0) {
        fprintf(stderr, "Failed to alloc output context: %s\n", av_err2str(ret));
        return 1;
    }

    AVDictionary *hls_opts = NULL;
    char *seg_pattern = make_segment_pattern(playlist);
    if (!seg_pattern) {
        fprintf(stderr, "Failed to build segment pattern.\n");
        return 1;
    }

    av_dict_set(&hls_opts, "hls_time", argv[3], 0);
    av_dict_set(&hls_opts, "hls_list_size", "0", 0);
    av_dict_set(&hls_opts, "hls_flags", "independent_segments", 0);
    av_dict_set(&hls_opts, "hls_playlist_type", "event", 0);
    av_dict_set(&hls_opts, "hls_segment_type", "fmp4", 0);
    av_dict_set(&hls_opts, "hls_fmp4_init_filename", "init.mp4", 0);
    av_dict_set(&hls_opts, "hls_segment_filename", seg_pattern, 0);

    // Video stream copy
    AVStream *in_vst = NULL;
    AVStream *out_vst = NULL;
    if (v_idx >= 0) {
        in_vst = ifmt->streams[v_idx];
        out_vst = avformat_new_stream(ofmt, NULL);
        if (!out_vst) {
            fprintf(stderr, "Failed to create video output stream.\n");
            return 1;
        }
        ret = avcodec_parameters_copy(out_vst->codecpar, in_vst->codecpar);
        if (ret < 0) {
            fprintf(stderr, "Failed to copy video codecpar: %s\n", av_err2str(ret));
            return 1;
        }
        out_vst->time_base = in_vst->time_base;

        if (out_vst->codecpar->codec_id == AV_CODEC_ID_HEVC) {
            out_vst->codecpar->codec_tag = MKTAG('h','v','c','1');
        }
    }

    // Audio re-encode to AAC
    AVStream *in_ast = NULL;
    AVStream *out_ast = NULL;
    if (a_idx >= 0) {
        in_ast = ifmt->streams[a_idx];
        if ((ret = open_audio_decoder(in_ast, &aud_dec_ctx)) < 0) {
            fprintf(stderr, "Failed to open audio decoder: %s\n", av_err2str(ret));
            return 1;
        }

        int in_rate = aud_dec_ctx->sample_rate;
        if ((ret = open_aac_encoder(&aud_enc_ctx, in_rate, audio_bitrate_kbps)) < 0) {
            fprintf(stderr, "Failed to open AAC encoder: %s\n", av_err2str(ret));
            return 1;
        }

        if (ofmt->oformat->flags & AVFMT_GLOBALHEADER) {
            aud_enc_ctx->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
        }

        out_ast = avformat_new_stream(ofmt, NULL);
        if (!out_ast) {
            fprintf(stderr, "Failed to create audio output stream.\n");
            return 1;
        }
        out_ast->time_base = aud_enc_ctx->time_base;
        ret = avcodec_parameters_from_context(out_ast->codecpar, aud_enc_ctx);
        if (ret < 0) {
            fprintf(stderr, "Failed to copy audio encoder params: %s\n", av_err2str(ret));
            return 1;
        }

        AVChannelLayout in_layout = aud_dec_ctx->ch_layout;
        if (in_layout.nb_channels == 0) {
            av_channel_layout_default(&in_layout, aud_dec_ctx->ch_layout.nb_channels > 0 ? aud_dec_ctx->ch_layout.nb_channels : 2);
        }

        ret = swr_alloc_set_opts2(&swr,
                                 &aud_enc_ctx->ch_layout, aud_enc_ctx->sample_fmt, aud_enc_ctx->sample_rate,
                                 &in_layout, aud_dec_ctx->sample_fmt, aud_dec_ctx->sample_rate,
                                 0, NULL);
        if (ret < 0 || !swr) {
            fprintf(stderr, "Failed to alloc resampler: %s\n", av_err2str(ret));
            return 1;
        }
        if ((ret = swr_init(swr)) < 0) {
            fprintf(stderr, "Failed to init resampler: %s\n", av_err2str(ret));
            return 1;
        }
    }

    if ((ret = avformat_write_header(ofmt, &hls_opts)) < 0) {
        fprintf(stderr, "Failed to write header: %s\n", av_err2str(ret));
        return 1;
    }
    av_dict_free(&hls_opts);

    pkt = av_packet_alloc();
    frame = av_frame_alloc();
    resampled = av_frame_alloc();
    if (!pkt || !frame || !resampled) {
        fprintf(stderr, "Out of memory.\n");
        return 1;
    }

    int64_t v_start = AV_NOPTS_VALUE;
    int64_t a_start = AV_NOPTS_VALUE;
    int64_t audio_pts = 0;

    while ((ret = av_read_frame(ifmt, pkt)) >= 0) {
        AVStream *in_st = ifmt->streams[pkt->stream_index];

        // Stop after max_seconds based on input timestamps
        if (max_seconds > 0 && pkt->pts != AV_NOPTS_VALUE) {
            AVRational tb = in_st->time_base;
            int64_t t = av_rescale_q(pkt->pts, tb, (AVRational){1,1});
            if (t >= max_seconds) {
                av_packet_unref(pkt);
                break;
            }
        }

        if (pkt->stream_index == v_idx && out_vst) {
            if (v_start == AV_NOPTS_VALUE && pkt->pts != AV_NOPTS_VALUE)
                v_start = pkt->pts;
            if (v_start != AV_NOPTS_VALUE) {
                if (pkt->pts != AV_NOPTS_VALUE) pkt->pts -= v_start;
                if (pkt->dts != AV_NOPTS_VALUE) pkt->dts -= v_start;
            }
            av_packet_rescale_ts(pkt, in_st->time_base, out_vst->time_base);
            pkt->stream_index = out_vst->index;
            ret = av_interleaved_write_frame(ofmt, pkt);
            av_packet_unref(pkt);
            if (ret < 0) {
                fprintf(stderr, "Error writing video packet: %s\n", av_err2str(ret));
                break;
            }
        } else if (pkt->stream_index == a_idx && out_ast) {
            ret = avcodec_send_packet(aud_dec_ctx, pkt);
            av_packet_unref(pkt);
            if (ret < 0) {
                fprintf(stderr, "Error sending audio packet: %s\n", av_err2str(ret));
                break;
            }

            while ((ret = avcodec_receive_frame(aud_dec_ctx, frame)) >= 0) {
                if (a_start == AV_NOPTS_VALUE && frame->pts != AV_NOPTS_VALUE)
                    a_start = frame->pts;
                if (a_start != AV_NOPTS_VALUE && frame->pts != AV_NOPTS_VALUE)
                    frame->pts -= a_start;

                resampled->channel_layout = aud_enc_ctx->ch_layout;
                resampled->sample_rate = aud_enc_ctx->sample_rate;
                resampled->format = aud_enc_ctx->sample_fmt;
                resampled->nb_samples = (int)av_rescale_rnd(
                    swr_get_delay(swr, aud_dec_ctx->sample_rate) + frame->nb_samples,
                    aud_enc_ctx->sample_rate, aud_dec_ctx->sample_rate, AV_ROUND_UP);

                ret = av_frame_get_buffer(resampled, 0);
                if (ret < 0) {
                    fprintf(stderr, "Error allocating resampled buffer: %s\n", av_err2str(ret));
                    break;
                }

                ret = swr_convert(swr, resampled->data, resampled->nb_samples,
                                  (const uint8_t **)frame->data, frame->nb_samples);
                if (ret < 0) {
                    fprintf(stderr, "Error resampling audio: %s\n", av_err2str(ret));
                    break;
                }
                resampled->nb_samples = ret;
                resampled->pts = audio_pts;
                audio_pts += resampled->nb_samples;

                ret = avcodec_send_frame(aud_enc_ctx, resampled);
                av_frame_unref(resampled);
                av_frame_unref(frame);
                if (ret < 0) {
                    fprintf(stderr, "Error sending audio frame: %s\n", av_err2str(ret));
                    break;
                }

                while ((ret = avcodec_receive_packet(aud_enc_ctx, pkt)) >= 0) {
                    av_packet_rescale_ts(pkt, aud_enc_ctx->time_base, out_ast->time_base);
                    pkt->stream_index = out_ast->index;
                    ret = av_interleaved_write_frame(ofmt, pkt);
                    av_packet_unref(pkt);
                    if (ret < 0) {
                        fprintf(stderr, "Error writing audio packet: %s\n", av_err2str(ret));
                        break;
                    }
                }
                if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) {
                    ret = 0;
                } else if (ret < 0) {
                    break;
                }
            }
            if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF) {
                ret = 0;
            } else if (ret < 0) {
                break;
            }
        } else {
            av_packet_unref(pkt);
        }

        if (ret < 0)
            break;
    }

    // Flush audio encoder
    if (aud_enc_ctx && out_ast) {
        ret = avcodec_send_frame(aud_enc_ctx, NULL);
        while (ret >= 0) {
            ret = avcodec_receive_packet(aud_enc_ctx, pkt);
            if (ret == AVERROR_EOF || ret == AVERROR(EAGAIN)) {
                break;
            }
            if (ret < 0) {
                fprintf(stderr, "Error flushing audio encoder: %s\n", av_err2str(ret));
                break;
            }
            av_packet_rescale_ts(pkt, aud_enc_ctx->time_base, out_ast->time_base);
            pkt->stream_index = out_ast->index;
            av_interleaved_write_frame(ofmt, pkt);
            av_packet_unref(pkt);
        }
    }

    av_write_trailer(ofmt);

    av_packet_free(&pkt);
    av_frame_free(&frame);
    av_frame_free(&resampled);
    swr_free(&swr);
    avcodec_free_context(&aud_dec_ctx);
    avcodec_free_context(&aud_enc_ctx);
    avformat_close_input(&ifmt);
    avformat_free_context(ofmt);
    av_free(seg_pattern);

    return ret < 0 ? 1 : 0;
}
