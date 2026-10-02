/* Kiểm tra trọn đường đi của ZKFinger SDK (libzkfp.so) với libsilkidcap.so thay thế:
 *   Init -> Open -> GetParameters -> chụp 3 lần (trích mẫu = gọi callback bản quyền
 *   sensorCheckLic) -> GenRegTemplate (ghép 3 mẫu) -> MatchFinger.
 * Chạy: LD_LIBRARY_PATH=<thư mục chứa libzkfp.so + libsilkidcap.so> ./zkfp-selftest [giây]
 * Dùng dlopen để không cần header của SDK. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef int (*fn_v)(void);
typedef void *(*fn_open)(int);
typedef int (*fn_h)(void *);
typedef void *(*fn_db)(void);
typedef int (*fn_getp)(void *, int, unsigned char *, unsigned int *);
typedef int (*fn_acq)(void *, unsigned char *, unsigned int, unsigned char *, unsigned int *);
typedef int (*fn_gen)(void *, unsigned char *, unsigned char *, unsigned char *, unsigned char *, unsigned int *);
typedef int (*fn_match)(void *, unsigned char *, unsigned int, unsigned char *, unsigned int);

static void *sym(void *lib, const char *name) {
    void *p = dlsym(lib, name);
    if (!p) { printf("THIẾU HÀM %s\n", name); exit(10); }
    return p;
}

static int get_int(fn_getp getp, void *h, int code) {
    unsigned char b[8] = {0}; unsigned int n = sizeof b;
    int r = getp(h, code, b, &n);
    return r == 0 ? *(int *)b : r;
}

static void get_str(fn_getp getp, void *h, int code, char *out, unsigned int cap) {
    unsigned int n = cap - 1; memset(out, 0, cap);
    if (getp(h, code, (unsigned char *)out, &n) != 0) strcpy(out, "(lỗi)");
}

int main(int argc, char **argv) {
    /* Tự đọc số thay vì atoi/strtol: với _GNU_SOURCE, header glibc >= 2.38 đổi chúng thành
     * __isoc23_strtol (GLIBC_2.38) -> không chạy được trên Ubuntu 22.04. */
    int secs = 0;
    for (const char *c = argc > 1 ? argv[1] : ""; *c >= '0' && *c <= '9'; c++) secs = secs * 10 + (*c - '0');
    if (secs <= 0) secs = 60;
    void *lib = dlopen("libzkfp.so", RTLD_NOW | RTLD_GLOBAL);
    if (!lib) { printf("Không nạp được libzkfp.so: %s\n", dlerror()); return 1; }

    Dl_info di;
    void *si = dlsym(RTLD_DEFAULT, "sensorInit");
    if (si && dladdr(si, &di)) printf("libsilkidcap đang dùng: %s\n", di.dli_fname);

    fn_v init = sym(lib, "ZKFPM_Init"), term = sym(lib, "ZKFPM_Terminate"), count = sym(lib, "ZKFPM_GetDeviceCount");
    fn_open open = sym(lib, "ZKFPM_OpenDevice");
    fn_h close = sym(lib, "ZKFPM_CloseDevice"), closedb = sym(lib, "ZKFPM_CloseDBCache");
    fn_db createdb = sym(lib, "ZKFPM_CreateDBCache");
    fn_getp getp = sym(lib, "ZKFPM_GetParameters");
    fn_acq acq = sym(lib, "ZKFPM_AcquireFingerprint");
    fn_gen gen = sym(lib, "ZKFPM_GenRegTemplate");
    fn_match match = sym(lib, "ZKFPM_MatchFinger");

    int r = init();
    printf("ZKFPM_Init = %d\n", r);
    if (r != 0 && r != 1) { printf("=> KHÔNG có đầu đọc / chưa có quyền USB.\n"); return 2; }
    printf("GetDeviceCount = %d\n", count());
    void *h = open(0);
    printf("OpenDevice(0) = %p\n", h);
    if (!h) { term(); return 3; }
    int w = get_int(getp, h, 1), ht = get_int(getp, h, 2), dpi = get_int(getp, h, 3);
    char m[64], model[64], sn[64];
    get_str(getp, h, 1101, m, sizeof m); get_str(getp, h, 1102, model, sizeof model); get_str(getp, h, 1103, sn, sizeof sn);
    printf("Ảnh %dx%d @%d dpi | %s %s SN %s | VID/PID=0x%08x\n", w, ht, dpi, m, model, sn, get_int(getp, h, 1015));

    void *db = createdb();
    unsigned char *img = malloc((size_t)w * ht);
    unsigned char tpl[3][2048]; unsigned int cb[3];
    int n = 0;
    printf("Chạm CÙNG MỘT ngón tay 3 lần (nhấc lên giữa các lần), chờ %d giây...\n", secs);
    fflush(stdout);
    time_t end = time(NULL) + secs;
    while (n < 3 && time(NULL) < end) {
        cb[n] = sizeof tpl[n];
        int a = acq(h, img, (unsigned int)w * ht, tpl[n], &cb[n]);
        if (a == 0) {
            char name[32]; snprintf(name, sizeof name, "selftest-%d.pgm", n + 1);
            FILE *f = fopen(name, "wb");
            if (f) { fprintf(f, "P5\n%d %d\n255\n", w, ht); fwrite(img, 1, (size_t)w * ht, f); fclose(f); }
            printf("  Lần %d: mẫu %u byte (ảnh -> %s)\n", n + 1, cb[n], name);
            fflush(stdout);
            n++;
            sleep(1);
        } else {
            usleep(200 * 1000);
        }
    }
    int ok = 0;
    if (n == 3) {
        int s12 = match(db, tpl[0], cb[0], tpl[1], cb[1]);
        int s13 = match(db, tpl[0], cb[0], tpl[2], cb[2]);
        unsigned char reg[2048]; unsigned int cbreg = sizeof reg;
        int g = gen(db, tpl[0], tpl[1], tpl[2], reg, &cbreg);
        printf("MatchFinger(1,2) = %d, MatchFinger(1,3) = %d (điểm > 0 là cùng ngón)\n", s12, s13);
        printf("GenRegTemplate = %d, mẫu đăng ký %u byte\n", g, cbreg);
        ok = g == 0 && s12 > 0;
    } else {
        printf("Chỉ chụp được %d/3 lần trong %d giây.\n", n, secs);
    }
    closedb(db);
    close(h);
    term();
    printf(ok ? "==> THÀNH CÔNG\n" : "==> CHƯA ĐẠT\n");
    return ok ? 0 : 4;
}
