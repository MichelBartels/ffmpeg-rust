#include <dirent.h>
#include <errno.h>
#include <ftw.h>
#include <limits.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "fftools/fftools_context.h"

#include "libavutil/avutil.h"
#include "libavutil/error.h"
#include "libavutil/mem.h"
#include "libavutil/sha.h"

#define DEFAULT_INPUT "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4"
#define DEFAULT_PROTO "myproto://bbb"
#define SEG_TIME 4
#define ABR_KBPS 128
#define MAX_SECONDS 600

typedef struct FileList {
    char **items;
    size_t nb;
    size_t cap;
} FileList;

typedef struct ThreadArgs {
    const char *input;
    const char *outdir;
    int ret;
    FftoolsContext ctx;
} ThreadArgs;

static int list_add(FileList *list, const char *relpath)
{
    if (list->nb == list->cap) {
        size_t new_cap = list->cap ? list->cap * 2 : 32;
        char **new_items = realloc(list->items, new_cap * sizeof(*new_items));
        if (!new_items)
            return AVERROR(ENOMEM);
        list->items = new_items;
        list->cap = new_cap;
    }
    list->items[list->nb] = strdup(relpath);
    if (!list->items[list->nb])
        return AVERROR(ENOMEM);
    list->nb++;
    return 0;
}

static int list_files(const char *base, const char *rel, FileList *list)
{
    char path[PATH_MAX];
    DIR *dir;
    struct dirent *ent;
    int ret = 0;

    if (snprintf(path, sizeof(path), "%s/%s", base, rel) >= (int)sizeof(path))
        return AVERROR(ENAMETOOLONG);

    dir = opendir(path);
    if (!dir)
        return AVERROR(errno);

    while ((ent = readdir(dir))) {
        struct stat st;
        char rel_path[PATH_MAX];
        char full_path[PATH_MAX];

        if (!strcmp(ent->d_name, ".") || !strcmp(ent->d_name, ".."))
            continue;

        if (!strcmp(rel, "."))
            snprintf(rel_path, sizeof(rel_path), "./%s", ent->d_name);
        else
            snprintf(rel_path, sizeof(rel_path), "%s/%s", rel, ent->d_name);

        if (snprintf(full_path, sizeof(full_path), "%s/%s", base, rel_path) >= (int)sizeof(full_path)) {
            ret = AVERROR(ENAMETOOLONG);
            break;
        }

        if (stat(full_path, &st) < 0) {
            ret = AVERROR(errno);
            break;
        }

        if (S_ISDIR(st.st_mode)) {
            ret = list_files(base, rel_path, list);
            if (ret < 0)
                break;
        } else if (S_ISREG(st.st_mode)) {
            ret = list_add(list, rel_path);
            if (ret < 0)
                break;
        }
    }

    closedir(dir);
    return ret;
}

static int cmp_str(const void *a, const void *b)
{
    const char *const *sa = a;
    const char *const *sb = b;
    return strcmp(*sa, *sb);
}

static void list_free(FileList *list)
{
    for (size_t i = 0; i < list->nb; i++)
        free(list->items[i]);
    free(list->items);
    list->items = NULL;
    list->nb = 0;
    list->cap = 0;
}

static int sha256_file(const char *path, uint8_t digest[32])
{
    uint8_t buf[16384];
    struct AVSHA *sha = NULL;
    FILE *f = NULL;
    size_t n;
    int ret = 0;

    f = fopen(path, "rb");
    if (!f)
        return AVERROR(errno);

    sha = av_sha_alloc();
    if (!sha) {
        ret = AVERROR(ENOMEM);
        goto out;
    }

    av_sha_init(sha, 256);
    while ((n = fread(buf, 1, sizeof(buf), f)) > 0)
        av_sha_update(sha, buf, n);

    if (ferror(f)) {
        ret = AVERROR(errno);
        goto out;
    }

    av_sha_final(sha, digest);

out:
    av_free(sha);
    fclose(f);
    return ret;
}

static void *run_ffmpeg_thread(void *arg)
{
    ThreadArgs *t = arg;
    char seg_path[PATH_MAX];
    char out_path[PATH_MAX];
    char seg_time[16];
    char abr_kbps[16];
    char max_seconds[16];

    snprintf(seg_time, sizeof(seg_time), "%d", SEG_TIME);
    snprintf(abr_kbps, sizeof(abr_kbps), "%dk", ABR_KBPS);
    snprintf(max_seconds, sizeof(max_seconds), "%d", MAX_SECONDS);

    snprintf(seg_path, sizeof(seg_path), "%s/seg_%%05d.m4s", t->outdir);
    snprintf(out_path, sizeof(out_path), "%s/out.m3u8", t->outdir);

    t->ctx.install_signal_handlers = 0;
    t->ctx.stdin_interaction = 0;

    char *argv[] = {
        (char *)"ffmpeg",
        (char *)"-hide_banner",
        (char *)"-loglevel",
        (char *)"error",
        (char *)"-y",
        (char *)"-fflags",
        (char *)"+genpts",
        (char *)"-i",
        (char *)t->input,
        (char *)"-c:v",
        (char *)"copy",
        (char *)"-tag:v",
        (char *)"hvc1",
        (char *)"-c:a",
        (char *)"aac",
        (char *)"-b:a",
        abr_kbps,
        (char *)"-ac",
        (char *)"2",
        (char *)"-f",
        (char *)"hls",
        (char *)"-hls_time",
        seg_time,
        (char *)"-hls_list_size",
        (char *)"0",
        (char *)"-hls_flags",
        (char *)"independent_segments",
        (char *)"-hls_playlist_type",
        (char *)"event",
        (char *)"-hls_segment_type",
        (char *)"fmp4",
        (char *)"-hls_fmp4_init_filename",
        (char *)"init.mp4",
        (char *)"-hls_segment_filename",
        seg_path,
        (char *)"-t",
        max_seconds,
        out_path,
    };

    t->ret = ffmpeg_run(&t->ctx, (int)(sizeof(argv) / sizeof(argv[0])), argv);
    return NULL;
}

static int compare_outputs(const char *direct, const char *proto)
{
    FileList a = {0}, b = {0};
    int ret;

    ret = list_files(direct, ".", &a);
    if (ret < 0)
        goto out;
    ret = list_files(proto, ".", &b);
    if (ret < 0)
        goto out;

    qsort(a.items, a.nb, sizeof(a.items[0]), cmp_str);
    qsort(b.items, b.nb, sizeof(b.items[0]), cmp_str);

    if (a.nb != b.nb) {
        ret = AVERROR(EINVAL);
        goto out;
    }

    for (size_t i = 0; i < a.nb; i++) {
        if (strcmp(a.items[i], b.items[i])) {
            ret = AVERROR(EINVAL);
            goto out;
        }
    }

    for (size_t i = 0; i < a.nb; i++) {
        uint8_t sha_a[32];
        uint8_t sha_b[32];
        char path_a[PATH_MAX];
        char path_b[PATH_MAX];

        if (snprintf(path_a, sizeof(path_a), "%s/%s", direct, a.items[i]) >= (int)sizeof(path_a) ||
            snprintf(path_b, sizeof(path_b), "%s/%s", proto, b.items[i]) >= (int)sizeof(path_b)) {
            ret = AVERROR(ENAMETOOLONG);
            goto out;
        }

        ret = sha256_file(path_a, sha_a);
        if (ret < 0)
            goto out;
        ret = sha256_file(path_b, sha_b);
        if (ret < 0)
            goto out;

        if (memcmp(sha_a, sha_b, sizeof(sha_a))) {
            ret = AVERROR(EINVAL);
            goto out;
        }
    }

    ret = 0;
out:
    list_free(&a);
    list_free(&b);
    return ret;
}

static int unlink_cb(const char *fpath, const struct stat *sb, int typeflag, struct FTW *ftwbuf)
{
    (void)sb;
    (void)ftwbuf;
    if (typeflag == FTW_DP || typeflag == FTW_D)
        return rmdir(fpath);
    return unlink(fpath);
}

static void cleanup_tree(const char *path)
{
    nftw(path, unlink_cb, 64, FTW_DEPTH | FTW_PHYS);
}

int main(void)
{
    const char *input = getenv("INPUT_FILE");
    const char *proto = getenv("PROTO_URL");
    char tmpdir[] = "/tmp/ffmpeg-parity-XXXXXX";
    char out_direct[PATH_MAX];
    char out_proto[PATH_MAX];
    pthread_t th_direct;
    pthread_t th_proto;
    ThreadArgs direct = {0};
    ThreadArgs proto_args = {0};
    int ret = 1;

    if (!input)
        input = DEFAULT_INPUT;
    if (!proto)
        proto = DEFAULT_PROTO;

    if (!mkdtemp(tmpdir)) {
        fprintf(stderr, "mkdtemp failed: %s\n", strerror(errno));
        return 1;
    }

    snprintf(out_direct, sizeof(out_direct), "%s/direct", tmpdir);
    snprintf(out_proto, sizeof(out_proto), "%s/proto", tmpdir);

    if (mkdir(out_direct, 0755) < 0 || mkdir(out_proto, 0755) < 0) {
        fprintf(stderr, "mkdir failed: %s\n", strerror(errno));
        goto out;
    }

    direct.input = input;
    direct.outdir = out_direct;
    direct.ctx = *fftools_default_context();

    proto_args.input = proto;
    proto_args.outdir = out_proto;
    proto_args.ctx = *fftools_default_context();

    if (pthread_create(&th_direct, NULL, run_ffmpeg_thread, &direct) != 0 ||
        pthread_create(&th_proto, NULL, run_ffmpeg_thread, &proto_args) != 0) {
        fprintf(stderr, "pthread_create failed\n");
        goto out;
    }

    pthread_join(th_direct, NULL);
    pthread_join(th_proto, NULL);

    if (direct.ret < 0 || proto_args.ret < 0) {
        fprintf(stderr, "ffmpeg_run failed: direct=%d proto=%d\n", direct.ret, proto_args.ret);
        goto out;
    }

    if (compare_outputs(out_direct, out_proto) < 0) {
        fprintf(stderr, "FAIL: concurrent parity mismatch\n");
        goto out;
    }

    printf("PASS: concurrent myproto output matches direct file output\n");
    ret = 0;

out:
    if (!getenv("KEEP_TMP"))
        cleanup_tree(tmpdir);
    else
        fprintf(stderr, "Keeping temp dir: %s\n", tmpdir);
    return ret;
}
