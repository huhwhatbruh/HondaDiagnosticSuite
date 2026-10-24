/* Native Linux J2534 (PassThru, v04.04) driver for the GODIAG GD101 (a rebadged VNCI Nano), raw CAN only.
 *
 * Protocol from GODIAG_PT32.dll v2.3 (RVAs in the project's research notes) and a byte trace of that DLL driving the
 * adapter on a 2015 Accord (2026-10-12). The adapter is a CDC ACM serial port; 2534 baud selects its J2534 mode.
 * Frame: [n] [2n payload bytes] [crc], n = 16-bit words, or [0x7F] [len lo] [len hi] [payload] [crc] for odd or long
 * payloads; bit 7 of the first byte = XOR-encrypted; crc = CRC-8 poly 0x07 over everything before it. A reply
 * echoes the command's first byte with 0x40 set. This driver never encrypts: the adapter then answers with bit 7
 * set and a zero key (measured). After GODIAG's own driver ran, the adapter keeps that session's key until it loses
 * power (measured), which this driver reports as an error instead of guessing the key.
 *
 * Implemented: Open/Close, Connect/Disconnect (CAN, 29- or 11-bit), one pass filter, WriteMsgs, ReadMsgs,
 * READ_VBATT and CLEAR_RX_BUFFER, ReadVersion, GetLastError. Port: PassThruOpen's name, else $GD101_PORT, else
 * /dev/gd101. Build: transport/j2534.rs gd101_library() runs
 * cc -shared -fPIC -O2 -fstack-protector-strong -D_FORTIFY_SOURCE=2 -Wl,-z,relro,-z,now -o libgd101_j2534.so gd101_j2534.c */
#include <asm/termbits.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

typedef unsigned long ULONG;
typedef struct {
    ULONG ProtocolID, RxStatus, TxFlags, Timestamp, DataSize, ExtraDataIndex;
    unsigned char Data[4128];
} PASSTHRU_MSG;

enum { OK = 0, ERR_NOT_SUPPORTED = 1, ERR_INVALID_CHANNEL_ID = 2, ERR_INVALID_PROTOCOL_ID = 3, ERR_NULL_PARAMETER = 4,
       ERR_FAILED = 7, ERR_DEVICE_NOT_CONNECTED = 8, ERR_TIMEOUT = 9, ERR_INVALID_MSG = 0x0A,
       ERR_EXCEEDED_LIMIT = 0x0C, ERR_DEVICE_IN_USE = 0x0E, ERR_INVALID_IOCTL_ID = 0x0F, ERR_BUFFER_EMPTY = 0x10,
       ERR_INVALID_FILTER_ID = 0x16, ERR_INVALID_DEVICE_ID = 0x1A };
#define CAN 5
#define CAN_29BIT_ID 0x100
#define PASS_FILTER 1
#define READ_VBATT 3
#define CLEAR_RX_BUFFER 8
#define DEVICE_ID 1
#define CHANNEL_ID 1
#define QUEUE 256                       /* received frames kept between ReadMsgs calls */

static int fd = -1, connected, filtered, ext;
static unsigned char seq;
static char last[80];
static struct { ULONG id, ts, ext; unsigned char n, data[8]; } q[QUEUE];
static int qhead, qlen;

static long fail(long code, const char *what) {
    snprintf(last, sizeof last, "%s", what);
    return code;
}

static unsigned char crc8(const unsigned char *p, int n) {
    unsigned c = 0;
    while (n--) {
        c ^= *p++;
        for (int i = 0; i < 8; i++) c = c & 0x80 ? ((c << 1) ^ 0x07) & 0xFF : (c << 1) & 0xFF;
    }
    return c;
}

static long now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000L + t.tv_nsec / 1000000;
}

static int send_frame(const unsigned char *p, int n) {
    unsigned char f[300];
    int h;
    if (n % 2 == 0 && n < 0xFE) { f[0] = n / 2; h = 1; }
    else { f[0] = 0x7F; f[1] = n & 0xFF; f[2] = n >> 8; h = 3; }
    memcpy(f + h, p, n);
    f[h + n] = crc8(f, h + n);
    return write(fd, f, h + n + 1) == h + n + 1 ? 0 : -1;
}

/* One frame from the adapter into p (payload, returns its length), or -1 on timeout. Bad CRCs are skipped. */
static unsigned char rbuf[8192];
static int rlen;
static int recv_frame(unsigned char *p, long deadline) {
    for (;;) {
        while (rlen > 0) {
            int lng = (rbuf[0] & 0x7F) == 0x7F, h = lng ? 3 : 1;
            if (rlen < h) break;
            int n = lng ? rbuf[1] | rbuf[2] << 8 : (rbuf[0] & 0x7F) * 2;
            if (n > 0x1005) { memmove(rbuf, rbuf + 1, --rlen); continue; }   /* resync, as the driver refuses */
            if (rlen < h + n + 1) break;
            int good = crc8(rbuf, h + n) == rbuf[h + n];
            if (good) memcpy(p, rbuf + h, n);
            rlen -= h + n + 1;
            memmove(rbuf, rbuf + h + n + 1, rlen);
            if (good) return n;
        }
        long left = deadline - now_ms();
        struct pollfd pf = {fd, POLLIN, 0};
        if (left <= 0 || poll(&pf, 1, left) <= 0) return -1;
        int r = read(fd, rbuf + rlen, sizeof rbuf - rlen);
        if (r <= 0) return -1;
        rlen += r;
    }
}

/* A received CAN frame: 43 01 00 <ts LE32> 10 <filter> <id LE32> <dlc> <data>. */
static void queue_rx(const unsigned char *p, int n) {
    if (n < 14 || p[13] > 8 || n < 14 + p[13]) return;
    if (qlen == QUEUE) { qhead = (qhead + 1) % QUEUE; qlen--; }        /* oldest dropped, like a full device buffer */
    int i = (qhead + qlen++) % QUEUE;
    q[i].ts = p[3] | p[4] << 8 | p[5] << 16 | (ULONG)p[6] << 24;
    q[i].id = p[9] | p[10] << 8 | p[11] << 16 | (ULONG)p[12] << 24;
    q[i].ext = q[i].id > 0x7FF || ext;
    q[i].n = p[13];
    memcpy(q[i].data, p + 14, p[13]);
}

/* Send a command and wait for its reply (first byte | 0x40, and for 0x43 the same sequence number); received CAN
 * frames that arrive meanwhile are queued. Returns the reply length or a negative J2534 status. */
static int command(const unsigned char *cmd, int n, unsigned char *reply, size_t cap, int ms) {
    unsigned char p[0x1006];
    if (send_frame(cmd, n)) return -fail(ERR_DEVICE_NOT_CONNECTED, "write to the adapter failed");
    long deadline = now_ms() + ms;
    for (;;) {
        int r = recv_frame(p, deadline);
        if (r < 0) return -fail(ERR_TIMEOUT, "no reply from the adapter");
        if (r >= 2 && p[0] == 0x43 && p[1] == 0x01) { queue_rx(p, r); continue; }
        if (r >= 1 && (p[0] & 0xF0) != 0x40)
            return -fail(ERR_FAILED, "adapter is in a GODIAG encrypted session: replug it");
        if (p[0] != (cmd[0] | 0x40) || (p[0] == 0x43 && (r < 4 || p[3] != cmd[3]))) continue;
        if ((size_t)r > cap) r = cap;               /* the adapter's length field is not trusted */
        memcpy(reply, p, r);
        return r;
    }
}

static long simple(const unsigned char *cmd, int n, const char *what) {
    unsigned char r[64];
    int got = command(cmd, n, r, sizeof r, 500);
    if (got < 0) return -got;
    if (got < 4 || r[2] || r[3]) return fail(ERR_FAILED, what);
    return OK;
}

long PassThruOpen(const void *name, ULONG *id) {
    if (!id) return ERR_NULL_PARAMETER;
    if (fd >= 0) return fail(ERR_DEVICE_IN_USE, "already open");
    const char *port = name ? name : getenv("GD101_PORT") ? getenv("GD101_PORT") : "/dev/gd101";
    fd = open(port, O_RDWR | O_NOCTTY | O_CLOEXEC);
    if (fd < 0) return fail(ERR_DEVICE_NOT_CONNECTED, strerror(errno));
    struct termios2 t;
    if (ioctl(fd, TCGETS2, &t) == 0) {                 /* raw 8N1 at 2534 baud, DTR on: the J2534 mode switch */
        t.c_iflag = t.c_oflag = t.c_lflag = 0;
        t.c_cflag = BOTHER | CS8 | CREAD | CLOCAL;
        t.c_ispeed = t.c_ospeed = 2534;
        memset(t.c_cc, 0, sizeof t.c_cc);
        ioctl(fd, TCSETS2, &t);
    }
    int dtr = TIOCM_DTR;
    ioctl(fd, TIOCMBIS, &dtr);
    ioctl(fd, TCFLSH, TCIOFLUSH);
    rlen = qlen = 0;
    unsigned char hello[] = {0x00, 0x01}, start[] = {0x00, 0x02}, r[64];
    long deadline = now_ms() + 3000;                   /* the DLL retries 00 01 for up to 3 s (sub_71d0) */
    int got = -1;
    while (got < 0 && now_ms() < deadline) got = command(hello, 2, r, sizeof r, 300);
    if (got < 0 || command(start, 2, r, sizeof r, 500) < 0) {
        close(fd);
        fd = -1;
        return fail(ERR_DEVICE_NOT_CONNECTED, "the adapter does not answer in J2534 mode");
    }
    *id = DEVICE_ID;
    return OK;
}

long PassThruClose(ULONG id) {
    if (id != DEVICE_ID || fd < 0) return ERR_INVALID_DEVICE_ID;
    close(fd);
    fd = -1;
    connected = filtered = 0;
    return OK;
}

long PassThruConnect(ULONG dev, ULONG protocol, ULONG flags, ULONG baud, ULONG *channel) {
    if (dev != DEVICE_ID || fd < 0) return ERR_INVALID_DEVICE_ID;
    if (!channel) return ERR_NULL_PARAMETER;
    if (protocol != CAN) return fail(ERR_INVALID_PROTOCOL_ID, "only raw CAN (protocol 5)");
    if (connected) return fail(ERR_EXCEEDED_LIMIT, "one channel at a time");
    /* 01 00 01 00 6e <baud LE32> 50 0f 01 + 8 x 00, as the DLL sends it for CAN and ISO 15765 alike */
    unsigned char c[20] = {0x01, 0x00, 0x01, 0x00, 0x6E, baud, baud >> 8, baud >> 16, baud >> 24, 0x50, 0x0F, 0x01};
    long rc = simple(c, sizeof c, "the adapter refused the CAN channel");
    if (rc) return rc;
    connected = 1;
    ext = !!(flags & CAN_29BIT_ID);
    *channel = CHANNEL_ID;
    return OK;
}

long PassThruDisconnect(ULONG ch) {
    if (ch != CHANNEL_ID || !connected) return ERR_INVALID_CHANNEL_ID;
    unsigned char c[4] = {0x01, 0x00, 0x00, 0x00};
    connected = filtered = 0;
    return simple(c, sizeof c, "disconnect refused");
}

static ULONG be32(const unsigned char *p) { return (ULONG)p[0] << 24 | p[1] << 16 | p[2] << 8 | p[3]; }

long PassThruStartMsgFilter(ULONG ch, ULONG type, const PASSTHRU_MSG *mask, const PASSTHRU_MSG *pattern,
                            const PASSTHRU_MSG *fc, ULONG *fid) {
    (void)fc;
    if (ch != CHANNEL_ID || !connected) return ERR_INVALID_CHANNEL_ID;
    if (!mask || !pattern || !fid) return ERR_NULL_PARAMETER;
    if (type != PASS_FILTER) return fail(ERR_NOT_SUPPORTED, "pass filters only");
    if (filtered) return fail(ERR_EXCEEDED_LIMIT, "one filter (the only kind traced)");
    if (mask->DataSize < 4 || pattern->DataSize < 4) return ERR_INVALID_MSG;
    ULONG m = be32(mask->Data), v = be32(pattern->Data);
    /* 01 04 00 00 <count> <mask LE32> <pattern LE32> 00 */
    unsigned char c[14] = {0x01, 0x04, 0x00, 0x00, 0x01, m, m >> 8, m >> 16, m >> 24, v, v >> 8, v >> 16, v >> 24};
    long rc = simple(c, sizeof c, "the adapter refused the filter");
    if (rc) return rc;
    filtered = 1;
    *fid = 1;
    return OK;
}

long PassThruStopMsgFilter(ULONG ch, ULONG fid) {
    if (ch != CHANNEL_ID || !connected) return ERR_INVALID_CHANNEL_ID;
    if (fid != 1 || !filtered) return ERR_INVALID_FILTER_ID;
    unsigned char c[6] = {0x01, 0x04, 0x00, 0x00, 0x00, 0x00};
    filtered = 0;
    return simple(c, sizeof c, "the adapter refused to clear the filter");
}

long PassThruWriteMsgs(ULONG ch, const PASSTHRU_MSG *msgs, ULONG *count, ULONG timeout) {
    if (ch != CHANNEL_ID || !connected) return ERR_INVALID_CHANNEL_ID;
    if (!msgs || !count) return ERR_NULL_PARAMETER;
    for (ULONG k = 0; k < *count; k++) {
        const PASSTHRU_MSG *m = &msgs[k];
        if (m->ProtocolID != CAN || m->DataSize < 4 || m->DataSize > 12) { *count = k; return ERR_INVALID_MSG; }
        ULONG id = be32(m->Data);
        int n = m->DataSize - 4;
        /* 03 00 00 <seq> <flags: 04 = 29-bit> <id LE32> <dlc> <data>; reply 43 00 00 <seq> 00 <ts LE32> 01 */
        unsigned char c[18] = {0x03, 0x00, 0x00, ++seq, m->TxFlags & CAN_29BIT_ID ? 0x04 : 0x00,
                               id, id >> 8, id >> 16, id >> 24, n};
        memcpy(c + 10, m->Data + 4, n);
        unsigned char r[64];
        int got = command(c, 10 + n, r, sizeof r, timeout ? timeout : 100);
        if (got < 0) { *count = k; return -got; }
    }
    return OK;
}

long PassThruReadMsgs(ULONG ch, PASSTHRU_MSG *msgs, ULONG *count, ULONG timeout) {
    if (ch != CHANNEL_ID || !connected) return ERR_INVALID_CHANNEL_ID;
    if (!msgs || !count) return ERR_NULL_PARAMETER;
    ULONG want = *count, got = 0;
    long deadline = now_ms() + timeout;
    unsigned char p[0x1006];
    while (got < want) {
        while (qlen && got < want) {
            PASSTHRU_MSG *m = &msgs[got++];
            int i = qhead;
            qhead = (qhead + 1) % QUEUE;
            qlen--;
            memset(m, 0, offsetof(PASSTHRU_MSG, Data));
            m->ProtocolID = CAN;
            m->RxStatus = q[i].ext ? CAN_29BIT_ID : 0;
            m->Timestamp = q[i].ts;
            m->DataSize = 4 + q[i].n;
            m->Data[0] = q[i].id >> 24; m->Data[1] = q[i].id >> 16; m->Data[2] = q[i].id >> 8; m->Data[3] = q[i].id;
            memcpy(m->Data + 4, q[i].data, q[i].n);
        }
        if (got == want) break;
        int r = recv_frame(p, deadline);
        if (r < 0) break;
        if (r >= 2 && p[0] == 0x43 && p[1] == 0x01) queue_rx(p, r);
    }
    *count = got;
    return got ? OK : timeout ? ERR_TIMEOUT : ERR_BUFFER_EMPTY;
}

long PassThruIoctl(ULONG id, ULONG ioctl_id, void *in, void *out) {
    (void)in;
    if (fd < 0) return ERR_INVALID_DEVICE_ID;
    if (ioctl_id == CLEAR_RX_BUFFER) { qlen = 0; return OK; }
    if (ioctl_id != READ_VBATT) return fail(ERR_INVALID_IOCTL_ID, "only READ_VBATT and CLEAR_RX_BUFFER");
    if (!out) return ERR_NULL_PARAMETER;
    (void)id;
    /* 02 00 10 00 (device command 0x14, sub_26609): reply 42 00 00 10 <mV LE32> ... */
    unsigned char c[4] = {0x02, 0x00, 0x10, 0x00}, r[64];
    int got = command(c, 4, r, sizeof r, 500);
    if (got < 0) return -got;
    if (got < 8 || r[3] != 0x10) return fail(ERR_FAILED, "unexpected voltage reply");
    *(ULONG *)out = r[4] | r[5] << 8 | r[6] << 16 | (ULONG)r[7] << 24;
    return OK;
}

long PassThruReadVersion(ULONG dev, char *fw, char *dll, char *api) {
    if (dev != DEVICE_ID || fd < 0) return ERR_INVALID_DEVICE_ID;
    if (!fw || !dll || !api) return ERR_NULL_PARAMETER;
    /* 00 03 -> 40 03 00 <minor> <major> <serial, 9 characters> ...; GODIAG prints SN:<8 chars>,FW:major.minor */
    unsigned char c[2] = {0x00, 0x03}, r[64];
    int got = command(c, 2, r, sizeof r, 500);
    if (got < 0) return -got;
    if (got < 14) return fail(ERR_FAILED, "short version reply");
    snprintf(fw, 80, "SN:%.8s,FW:%u.%u", (char *)r + 5, r[4], r[3]);
    snprintf(dll, 80, "hondascan gd101 1.0");
    snprintf(api, 80, "04.04");
    return OK;
}

long PassThruGetLastError(char *text) {
    if (!text) return ERR_NULL_PARAMETER;
    snprintf(text, 80, "%s", last);
    return OK;
}

static long unsupported(void) { return fail(ERR_NOT_SUPPORTED, "not supported by this driver"); }
long PassThruSetProgrammingVoltage(ULONG d, ULONG p, ULONG v) { (void)d; (void)p; (void)v; return unsupported(); }
long PassThruStartPeriodicMsg(ULONG c, const PASSTHRU_MSG *m, ULONG *i, ULONG t) {
    (void)c; (void)m; (void)i; (void)t;
    return unsupported();
}
long PassThruStopPeriodicMsg(ULONG c, ULONG i) { (void)c; (void)i; return unsupported(); }
