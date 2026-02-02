/*
 * Copyright (c) The FFmpeg developers
 *
 * This file is part of FFmpeg.
 *
 * FFmpeg is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * FFmpeg is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with FFmpeg; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA
 */

#include <stdarg.h>
#include <string.h>

#include "avtextwriters.h"
#include "libavutil/bprint.h"
#include "libavutil/error.h"
#include "libavutil/mem.h"
#include "libavutil/opt.h"

#define WRITER_NAME "callbackwriter"

typedef struct CallbackWriterContext {
    const AVClass *class;
    AVTextWriterWriteCallback write_cb;
    void *opaque;
} CallbackWriterContext;

static const char *callbackwriter_get_name(void *ctx)
{
    return WRITER_NAME;
}

static const AVClass callbackwriter_class = {
    .class_name = WRITER_NAME,
    .item_name  = callbackwriter_get_name,
};

static void callback_w8(AVTextWriterContext *wctx, int b)
{
    CallbackWriterContext *ctx = wctx->priv;
    uint8_t ch = (uint8_t)b;

    if (ctx->write_cb)
        ctx->write_cb(ctx->opaque, &ch, 1);
}

static void callback_put_str(AVTextWriterContext *wctx, const char *str)
{
    CallbackWriterContext *ctx = wctx->priv;

    if (!ctx->write_cb || !str)
        return;

    ctx->write_cb(ctx->opaque, (const uint8_t *)str, strlen(str));
}

static void callback_vprintf(AVTextWriterContext *wctx, const char *fmt, va_list vl)
{
    CallbackWriterContext *ctx = wctx->priv;
    AVBPrint buf;

    if (!ctx->write_cb || !fmt)
        return;

    av_bprint_init(&buf, 0, AV_BPRINT_SIZE_UNLIMITED);
    av_vbprintf(&buf, fmt, vl);
    if (av_bprint_is_complete(&buf) && buf.len > 0)
        ctx->write_cb(ctx->opaque, (const uint8_t *)buf.str, buf.len);
    av_bprint_finalize(&buf, NULL);
}

const AVTextWriter avtextwriter_callback = {
    .name           = WRITER_NAME,
    .priv_size      = sizeof(CallbackWriterContext),
    .priv_class     = &callbackwriter_class,
    .writer_put_str = callback_put_str,
    .writer_vprintf = callback_vprintf,
    .writer_w8      = callback_w8,
};

int avtextwriter_create_callback(AVTextWriterContext **pwctx, AVTextWriterWriteCallback cb, void *opaque)
{
    CallbackWriterContext *ctx;
    int ret;

    if (!cb)
        return AVERROR(EINVAL);

    ret = avtextwriter_context_open(pwctx, &avtextwriter_callback);
    if (ret < 0)
        return ret;

    ctx = (*pwctx)->priv;
    ctx->write_cb = cb;
    ctx->opaque = opaque;

    return 0;
}
