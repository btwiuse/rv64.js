/* Freestanding fixed-work compute benchmark (no libc). Runs as PID 1 in an
 * initramfs on both x86-64 (v86) and riscv64 (rv64.js). Identical work; the
 * host harness times wall-clock from boot to the BENCH_DONE line. Folds all
 * kernel results into one hex checksum printed for cross-build verification. */
typedef unsigned long long u64; typedef unsigned int u32; /* u64 is 64-bit on BOTH ISAs (i386 unsigned long is 32-bit) — PERFORMANCE_PROGRESS.md cross-ISA equivalence */

#if defined(__x86_64__)
#define SYS_write 1
#define SYS_exit  60
static inline long sc3(long n, long a, long b, long c){
    long r; register long r10 asm("r10")=0;
    asm volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10):"rcx","r11","memory");
    return r;
}
#elif defined(__i386__)
#define SYS_write 4
#define SYS_exit  1
static inline long sc3(long n, long a, long b, long c){
    long r; asm volatile("int $0x80":"=a"(r):"a"(n),"b"(a),"c"(b),"d"(c):"memory"); return r;
}
#elif defined(__riscv)
#define SYS_write 64
#define SYS_exit  93
static inline long sc3(long n, long a, long b, long c){
    register long a7 asm("a7")=n,a0 asm("a0")=a,a1 asm("a1")=b,a2 asm("a2")=c;
    asm volatile("ecall":"+r"(a0):"r"(a7),"r"(a1),"r"(a2):"memory");
    return a0;
}
#endif
#ifndef WORK
#define WORK 1
#endif
static void wr(const char*s,long n){ sc3(SYS_write,1,(long)s,n); }
static void wrhex(u64 v){
    char b[19]; b[0]='0'; b[1]='x'; for(int i=0;i<16;i++){ int nib=(v>>((15-i)*4))&0xf; b[2+i]=nib<10?'0'+nib:'a'+nib-10; } b[18]='\n'; wr(b,19);
}

void bench_main(void){
    const int N=2048;
    const int SORT_ITERS=60*WORK; const long FP_ITERS=40000000L*WORK; const long MEM_ITERS=20000000L*WORK;
    static int a[2048]; static double d[2048];
    wr("BENCH_START\n",12);
    u64 isum=0; u32 rng=2463534242u;
    for(int it=0;it<SORT_ITERS;it++){
        for(int i=0;i<N;i++){ rng^=rng<<13; rng^=rng>>17; rng^=rng<<5; a[i]=(int)rng; }
        for(int i=1;i<N;i++){ int k=a[i],j=i-1; while(j>=0&&a[j]>k){a[j+1]=a[j];j--;} a[j+1]=k; }
        isum += (u32)a[0]^(u32)a[N/2]^(u32)a[N-1];
    }
    double f0=1.0,f1=0.5;
    for(long i=0;i<FP_ITERS;i++){ f0=f0*0.999999977+f1*1.0000001; f1=f1*0.999999983-f0*0.00000005+1.0; if(f0>1e6)f0*=1e-6; if(f1>1e6)f1*=1e-6; }
    for(int i=0;i<N;i++) d[i]=(double)a[i];
    double msum=0;
    for(long i=0;i<MEM_ITERS;i++){ int idx=(int)((u32)(i*2654435761u)>>21)&(N-1); d[idx]=d[idx]*1.0000001+1.0; msum+=d[idx]; }
    union{double dd;u64 uu;}u0={f0},u1={f1},u2={msum};
    isum ^= u0.uu ^ (u1.uu*3) ^ (u2.uu*7);
    wr("checksum=",9); wrhex(isum); wr("BENCH_DONE\n",11);
    sc3(SYS_exit,0,0,0);
    for(;;){}
}

#if defined(__riscv)
__attribute__((naked)) void _start(void){
    __asm__ volatile(".option push\n.option norelax\nlla gp, __global_pointer$\n.option pop\nandi sp, sp, -16\ncall bench_main\n");
}
#elif defined(__x86_64__)
__attribute__((naked)) void _start(void){
    __asm__ volatile("andq $-16, %rsp\ncall bench_main\n");
}
#elif defined(__i386__)
__attribute__((naked)) void _start(void){
    __asm__ volatile("andl $-16, %esp\ncall bench_main\n");
}
#endif
