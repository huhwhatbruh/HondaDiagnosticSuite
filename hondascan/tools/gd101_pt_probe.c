/* Research only (the driver itself is native: drivers/gd101_j2534.c): load GODIAG's J2534 DLL under
 * Wine and make the calls hondascan's j2534 backend makes (raw CAN 29-bit 500k, pass filter 0x18DAF100/0x1FFFFF00,
 * READ_VBATT, 3E 00 and 22 F1 90 with our own flow control), printing every return code and the session's XOR key,
 * so tools/gd101_wine_shim.c can log and the key decode every byte. Read-only on the bus. It also dumps the
 * unpacked DLL while loaded (gd101-pt-dump). Wine needs the device's FriendlyName "... (COM1)" under
 * HKLM\System\CurrentControlSet\Enum\USB\VID_E327&PID_2534\<serial> and HKLM\Software\Wine\Ports COM1 =
 * /dev/ttyACM0. After a run the adapter keeps the session key until it loses power: replug it before native use.
 *
 * Build without MinGW (clang + lld; import libraries from .def files listing the imports used here):
 *   llvm-dlltool -m i386 -d kernel32.def -l kernel32.lib -k; llvm-dlltool -m i386 -d msvcrt.def -l msvcrt.lib
 *   clang --target=i686-pc-windows-msvc -O1 -fno-builtin -mno-stack-arg-probe -fno-stack-protector -c gd101_pt_probe.c
 *   lld-link /subsystem:console /entry:mainCRTStartup /nodefaultlib gd101_pt_probe.obj kernel32.lib msvcrt.lib
 *
 * Usage under Wine: gd101_pt_probe.exe <path to GODIAG_PT32.dll> [ecu hex, default 0E] [pause seconds, default 0] [kline]
 * With `kline` as the fourth argument it makes the K-line calls instead of the CAN ones (ISO 9141 and ISO 14230 at 10400 baud:
 * connect, SET_CONFIG, pass-all filter, 5-baud init, fast init, one write, a read, disconnect), for tracing GODIAG's driver
 * against tools/gd101_fake.py (no adapter or car needed: the fake logs what the DLL sends).
 * The pause comes after PassThruOpen and again at the end, with the DLL loaded: the window for a memory dump.
 */
typedef unsigned long ULONG;
typedef void *HMODULE;
#define WINAPI __stdcall

__declspec(dllimport) HMODULE WINAPI LoadLibraryA(const char *);
__declspec(dllimport) unsigned char *WINAPI GetProcAddress(HMODULE, const char *);
__declspec(dllimport) void WINAPI Sleep(ULONG);
__declspec(dllimport) void WINAPI ExitProcess(unsigned);
__declspec(dllimport) int WINAPI VirtualProtect(void *, ULONG, ULONG, ULONG *);
__declspec(dllimport) ULONG WINAPI GetCurrentProcessId(void);
__declspec(dllimport) char *WINAPI GetCommandLineA(void);
__declspec(dllimport) int printf(const char *, ...);
__declspec(dllimport) int fflush(void *);
__declspec(dllimport) unsigned long strtoul(const char *, char **, int);

/* SAE J2534-1 v04.04 */
typedef struct {
    ULONG ProtocolID, RxStatus, TxFlags, Timestamp, DataSize, ExtraDataIndex;
    unsigned char Data[4128];
} PASSTHRU_MSG;
#define ISO15765 6
#define CAN_29BIT_ID 0x100
#define ISO15765_FRAME_PAD 0x40
#define FLOW_CONTROL_FILTER 3
#define CLEAR_RX_BUFFER 8

typedef long (WINAPI *Open_t)(void *, ULONG *);
typedef long (WINAPI *Close_t)(ULONG);
typedef long (WINAPI *Connect_t)(ULONG, ULONG, ULONG, ULONG, ULONG *);
typedef long (WINAPI *Disconnect_t)(ULONG);
typedef long (WINAPI *ReadVersion_t)(ULONG, char *, char *, char *);
typedef long (WINAPI *GetLastError_t)(char *);
typedef long (WINAPI *Filter_t)(ULONG, ULONG, PASSTHRU_MSG *, PASSTHRU_MSG *, PASSTHRU_MSG *, ULONG *);
typedef long (WINAPI *Write_t)(ULONG, PASSTHRU_MSG *, ULONG *, ULONG);
typedef long (WINAPI *Read_t)(ULONG, PASSTHRU_MSG *, ULONG *, ULONG);
typedef long (WINAPI *Ioctl_t)(ULONG, ULONG, void *, void *);

static GetLastError_t last_error;

__declspec(dllimport) int WINAPI DeviceIoControl(void *, ULONG, void *, ULONG, void *, ULONG, ULONG *, void *);
static int WINAPI my_timeouts(void *h, ULONG *t) {
    ULONG n, v[5] = {0xffffffff, 0, 0, t[3], t[4]};
    printf("SetCommTimeouts %lx %lx %lx %lx %lx -> MAXDWORD 0 0\n", t[0], t[1], t[2], t[3], t[4]);
    return DeviceIoControl(h, 0x1b001c, v, sizeof v, 0, 0, &n, 0);
}

static long report(const char *call, long rc) {
    char err[80] = "";
    if (rc && last_error) last_error(err);
    printf("%s = %ld%s%s\n", call, rc, rc ? "  " : "", err);
    fflush(0);
    return rc;
}

static void id_msg(PASSTHRU_MSG *m, ULONG id, const unsigned char *data, ULONG n) {
    unsigned char *p = m->Data;
    m->ProtocolID = ISO15765; m->RxStatus = 0; m->TxFlags = CAN_29BIT_ID | ISO15765_FRAME_PAD;
    m->Timestamp = 0; m->ExtraDataIndex = 0; m->DataSize = 4 + n;
    p[0] = id >> 24; p[1] = id >> 16; p[2] = id >> 8; p[3] = id;
    for (ULONG i = 0; i < n; i++) p[4 + i] = data[i];
}

static void pause(ULONG secs) {
    if (!secs) return;
    printf("PAUSE pid=%lu %lus\n", GetCurrentProcessId(), secs);
    fflush(0);
    Sleep(secs * 1000);
}

static void request(ULONG ch, Write_t wr, Read_t rd, ULONG tx, ULONG rx, const unsigned char *req, ULONG n) {
    PASSTHRU_MSG m, in;
    ULONG count = 1;
    id_msg(&m, tx, req, n);
    if (report("PassThruWriteMsgs", wr(ch, &m, &count, 500))) return;
    for (int tries = 0; tries < 5; tries++) {
        count = 1;
        long rc = rd(ch, &in, &count, 500);
        if (rc || !count) { report("PassThruReadMsgs", rc); continue; }
        printf("  rx status=%lx size=%lu:", in.RxStatus, in.DataSize);
        for (ULONG i = 0; i < in.DataSize && i < 64; i++) printf(" %02X", in.Data[i]);
        printf("\n");
        fflush(0);
        ULONG id = (ULONG)in.Data[0] << 24 | in.Data[1] << 16 | in.Data[2] << 8 | in.Data[3];
        if (id == rx && in.DataSize > 4 && !(in.RxStatus & 0x9)) return;   /* not a TX echo or first-frame note */
    }
}


/* K-line: what hondascan's gd101-j2534 driver would need to send for ISO 9141 / ISO 14230 (J2534-1 ids 3 and 4). */
typedef struct { ULONG Parameter, Value; } SCONFIG;
typedef struct { ULONG NumOfParams; SCONFIG *ConfigPtr; } SCONFIG_LIST;
typedef struct { ULONG NumOfBytes; unsigned char *BytePtr; } SBYTE_ARRAY;

static void kmsg(PASSTHRU_MSG *m, ULONG proto, ULONG flags, const unsigned char *d, ULONG n) {
    m->ProtocolID = proto; m->RxStatus = 0; m->TxFlags = flags; m->Timestamp = 0; m->ExtraDataIndex = 0; m->DataSize = n;
    for (ULONG i = 0; i < n; i++) m->Data[i] = d[i];
}

static void kline(ULONG dev, ULONG proto, ULONG flags, Connect_t Connect, Disconnect_t Disconnect, Filter_t StartMsgFilter,
                  Write_t WriteMsgs, Read_t ReadMsgs, Ioctl_t Ioctl) {
    ULONG ch = 0, fid = 0;
    printf("== K-line protocol %lu flags %lx\n", proto, flags);
    if (report("PassThruConnect K-line 10400", Connect(dev, proto, flags, 10400, &ch))) return;
    SCONFIG c[1] = {{0x1, 10400}};                                  /* DATA_RATE */
    SCONFIG_LIST cl = {1, c};
    report("PassThruIoctl SET_CONFIG DATA_RATE", Ioctl(ch, 2, &cl, 0));
    PASSTHRU_MSG mask, pat; unsigned char none[1] = {0};
    kmsg(&mask, proto, 0, none, 1); kmsg(&pat, proto, 0, none, 1);
    report("PassThruStartMsgFilter pass-all", StartMsgFilter(ch, 1, &mask, &pat, 0, &fid));
    ULONG mv = 0;
    report("PassThruIoctl READ_VBATT", Ioctl(ch, 3, 0, &mv)); printf("  vbatt %lu mV\n", mv);
    {   unsigned char addr = 0x01, out[8]; SBYTE_ARRAY in = {1, &addr}, o = {0, out};
        report("PassThruIoctl FIVE_BAUD_INIT 0x01", Ioctl(ch, 4, &in, &o));
        printf("  keybytes %lu\n", o.NumOfBytes);
    }
    {   unsigned char sc[] = {0xC1, 0x33, 0xF1, 0x81}; PASSTHRU_MSG in, out;
        kmsg(&in, proto, 0, sc, 4); kmsg(&out, proto, 0, none, 0);
        report("PassThruIoctl FAST_INIT C1 33 F1 81", Ioctl(ch, 5, &in, &out));
    }
    {   unsigned char req[] = {0xC2, 0x33, 0xF1, 0x3E, 0x00}; PASSTHRU_MSG m, r; ULONG count = 1;
        kmsg(&m, proto, 0, req, 5);
        report("PassThruWriteMsgs", WriteMsgs(ch, &m, &count, 100));
        count = 1;
        report("PassThruReadMsgs", ReadMsgs(ch, &r, &count, 150));
    }
    report("PassThruIoctl CLEAR_RX_BUFFER", Ioctl(ch, 8, 0, 0));
    report("PassThruDisconnect", Disconnect(ch));
}

static int run(int argc, char **argv) {
    if (argc < 2) { printf("usage: gd101_pt_probe <GODIAG_PT32.dll> [ecu hex] [pause s]\n"); return 2; }
    ULONG ecu = argc > 2 ? strtoul(argv[2], 0, 16) : 0x0E, secs = argc > 3 ? strtoul(argv[3], 0, 10) : 0;
    HMODULE dll = LoadLibraryA(argv[1]);
    printf("LoadLibrary %s -> %p\n", argv[1], dll);
    if (!dll) return 1;
    {   /* Wine workaround: read timeouts MAXDWORD/MAXDWORD/10 never complete under Wine 11; hook SetCommTimeouts */
        HMODULE kb = LoadLibraryA("kernelbase.dll");
        unsigned char *f = GetProcAddress(kb, "SetCommTimeouts"); ULONG old;
        int ok = VirtualProtect(f, 8, 0x40, &old);
        long rel = (long)((unsigned char *)my_timeouts - (f + 5));
        if (ok) { f[0] = 0xe9; f[1] = rel; f[2] = rel >> 8; f[3] = rel >> 16; f[4] = rel >> 24; }
        printf("hooked SetCommTimeouts %d\n", ok);
    }
#define FN(t, n) t n = (t)GetProcAddress(dll, "PassThru" #n)
    FN(Open_t, Open); FN(Close_t, Close); FN(Connect_t, Connect); FN(Disconnect_t, Disconnect);
    FN(ReadVersion_t, ReadVersion); FN(Filter_t, StartMsgFilter); FN(Write_t, WriteMsgs); FN(Read_t, ReadMsgs);
    FN(Ioctl_t, Ioctl);
    last_error = (GetLastError_t)GetProcAddress(dll, "PassThruGetLastError");
    if (!Open || !Connect || !WriteMsgs || !ReadMsgs || !StartMsgFilter) { printf("missing exports\n"); return 1; }

    ULONG dev = 0, ch = 0, fid = 0;
    long rc = report("PassThruOpen", Open(0, &dev));
    pause(secs);
    if (rc) return 1;
    {   unsigned char **g = *(unsigned char ***)((unsigned char *)dll + 0x31a30), *t = *(unsigned char **)g[1];
        printf("encrypt %u key", t[0x2b0]);          /* receive key[0x3b1 + (i & 31)], send key[0x3d0 - (i & 31)] */
        for (int i = 0x3b1; i <= 0x3d0; i++) printf(" %02x", t[i]);
        printf("\n"); fflush(0);
    }
    if (argc > 4) {
        kline(dev, 3, 0, Connect, Disconnect, StartMsgFilter, WriteMsgs, ReadMsgs, Ioctl);
        kline(dev, 4, 0, Connect, Disconnect, StartMsgFilter, WriteMsgs, ReadMsgs, Ioctl);
        report("PassThruClose", Close(dev));
        return 0;
    }
    char fw[80] = "", dllv[80] = "", api[80] = "";
    if (!report("PassThruReadVersion", ReadVersion(dev, fw, dllv, api)))
        printf("  firmware %s, dll %s, api %s\n", fw, dllv, api);
        typedef long (WINAPI *Stop_t)(ULONG, ULONG);
    Stop_t Stop = (Stop_t)GetProcAddress(dll, "PassThruStopMsgFilter");
    if (!report("PassThruConnect CAN 29-bit 500k", Connect(dev, 5, CAN_29BIT_ID, 500000, &ch))) {
        PASSTHRU_MSG mask, pat; unsigned char none[1];
        id_msg(&mask, 0x1FFFFF00, none, 0); id_msg(&pat, 0x18DAF100, none, 0);
        mask.ProtocolID = pat.ProtocolID = 5; mask.TxFlags = pat.TxFlags = CAN_29BIT_ID;
        report("PassThruStartMsgFilter pass", StartMsgFilter(ch, 1, &mask, &pat, 0, &fid));
        ULONG mv = 0;
        report("PassThruIoctl READ_VBATT", Ioctl(ch, 3, 0, &mv)); printf("  vbatt %lu mV\n", mv);
        static const unsigned char f1[] = {0x02,0x3E,0x00,0x55,0x55,0x55,0x55,0x55},
            f2[] = {0x03,0x22,0xF1,0x90,0x55,0x55,0x55,0x55}, fc[] = {0x30,0x00,0x00,0x55,0x55,0x55,0x55,0x55};
        const unsigned char *fr[] = {f1, f2, fc};
        for (int k = 0; k < 3; k++) {
            PASSTHRU_MSG m, in; ULONG count = 1;
            id_msg(&m, 0x18DA0EF1, fr[k], 8); m.ProtocolID = 5; m.TxFlags = CAN_29BIT_ID;
            report("PassThruWriteMsgs raw", WriteMsgs(ch, &m, &count, 100));
            for (int t = 0; t < 8; t++) {
                count = 1; long rc = ReadMsgs(ch, &in, &count, 150);
                if (rc || !count) break;
                printf("  rx status=%lx size=%lu:", in.RxStatus, in.DataSize);
                for (ULONG i = 0; i < in.DataSize && i < 16; i++) printf(" %02X", in.Data[i]);
                printf("\n"); fflush(0);
            }
        }
        report("PassThruStopMsgFilter", Stop(ch, fid));
        report("PassThruDisconnect", Disconnect(ch));
    }
    report("PassThruClose", Close(dev));
    return 0;
}

/* No CRT: split the command line by hand (quotes group, no escapes; enough for paths). */
void mainCRTStartup(void) {
    static char buf[1024], *argv[8];
    int argc = 0, q = 0;
    char *s = GetCommandLineA(), *d = buf;
    while (*s && argc < 8 && d < buf + sizeof buf - 2) {
        while (*s == ' ') s++;
        if (!*s) break;
        argv[argc++] = d;
        for (; *s && (q || *s != ' '); s++) {
            if (*s == '"') q = !q; else *d++ = *s;
        }
        *d++ = 0;
    }
    ExitProcess(run(argc, argv));
}
