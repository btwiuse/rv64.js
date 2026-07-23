/* rv64.js virt-machine smoke/regression init.
 *
 * Runs as PID 1 in a tiny initramfs. Deliberately exercises the three
 * full-system paths that were broken (and are now fixed):
 *
 *   1. 8250 THRE (TX) interrupt   -> a big console-output burst + tty drain.
 *      The 8250 driver goes interrupt-driven once its buffer fills; without a
 *      THR-empty interrupt the drain (and the writer) block forever.
 *   2. LR/SC reservation on trap  -> a fork+exec loop while the timer ticks.
 *      Atomics (refcounts, runqueue) taken across interrupts must not lose
 *      updates, or a task is stranded and the whole system idles.
 *   3. rdtime advancing           -> nanosleep + a rdtime-delta check.
 *
 * On success it prints SMOKE_OK and powers off. If any path is broken the
 * guest wedges and the harness times out -> test failure.
 */
#include <unistd.h>
#include <termios.h>
#include <sys/wait.h>
#include <sys/reboot.h>
#include <sys/mount.h>
#include <time.h>
#include <string.h>

static void emit(const char *s) { write(1, s, strlen(s)); }

static unsigned long rdtime(void) {
    unsigned long t;
    __asm__ volatile("rdtime %0" : "=r"(t));
    return t;
}

int main(int argc, char **argv) {
    if (argc > 1 && !strcmp(argv[1], "child")) {
        /* child: a little CPU work, then exit (exercises fork/exec/wait) */
        volatile long x = 0;
        for (int i = 0; i < 200000; i++) x += i;
        _exit((int)(x & 7));
    }

    emit("SMOKE_START\n");

    /* (3) rdtime must advance across a sleep */
    unsigned long t0 = rdtime();
    struct timespec ts = {0, 30 * 1000 * 1000};
    nanosleep(&ts, 0);
    if (rdtime() == t0) { emit("FAIL_RDTIME_STUCK\n"); reboot(RB_POWER_OFF); }
    emit("RDTIME_OK\n");

    /* (1) flood the console then drain -> forces interrupt-driven TX / THRE */
    for (int i = 0; i < 400; i++)
        emit("filling the 8250 transmit buffer to force interrupt-driven TX ...\n");
    struct termios t;
    if (tcgetattr(1, &t) == 0) tcsetattr(1, TCSADRAIN, &t);
    tcdrain(1);
    emit("TTY_DRAIN_OK\n");

    /* (2) fork+exec loop with the timer ticking underneath */
    for (int i = 0; i < 40; i++) {
        pid_t p = fork();
        if (p == 0) {
            char *a[] = {"/init", "child", 0};
            execv("/init", a);
            _exit(127);
        }
        int st;
        waitpid(p, &st, 0);
    }
    emit("FORKS_OK\n");

    emit("SMOKE_OK\n");
    sync();
    reboot(RB_POWER_OFF);
    for (;;) pause();
    return 0;
}
