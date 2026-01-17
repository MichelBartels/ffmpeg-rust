#include "libavformat/avio.h"
#include "libavformat/url.h"
#include "libavutil/error.h"
#include "libavutil/mem.h"

// Rust ABI
void *rsproto_open(const char *uri, int flags, int *is_streamed);
int rsproto_read(void *ctx, unsigned char *buf, int size);
int64_t rsproto_seek(void *ctx, int64_t pos, int whence);
int rsproto_close(void *ctx);

typedef struct MyProtoContext {
    void *rctx;
} MyProtoContext;

static int myproto_open(URLContext *h, const char *uri, int flags)
{
    MyProtoContext *c = h->priv_data;
    int is_streamed = 0;

    c->rctx = rsproto_open(uri, flags, &is_streamed);
    if (!c->rctx)
        return AVERROR(EIO);

    h->is_streamed = is_streamed;
    return 0;
}

static int myproto_read(URLContext *h, unsigned char *buf, int size)
{
    MyProtoContext *c = h->priv_data;
    if (!c || !c->rctx)
        return AVERROR(EIO);
    return rsproto_read(c->rctx, buf, size);
}

static int64_t myproto_seek(URLContext *h, int64_t pos, int whence)
{
    MyProtoContext *c = h->priv_data;
    if (!c || !c->rctx)
        return AVERROR(EIO);
    return rsproto_seek(c->rctx, pos, whence);
}

static int myproto_close(URLContext *h)
{
    MyProtoContext *c = h->priv_data;
    if (c && c->rctx) {
        rsproto_close(c->rctx);
        c->rctx = NULL;
    }
    return 0;
}

const URLProtocol ff_myproto_protocol = {
    .name            = "myproto",
    .url_open        = myproto_open,
    .url_read        = myproto_read,
    .url_seek        = myproto_seek,
    .url_close       = myproto_close,
    .priv_data_size  = sizeof(MyProtoContext),
};
