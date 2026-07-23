/* Pure-ALU integer benchmark (JIT-friendly: register xorshift, no FP, no
 * data-dependent branches). 32-bit ops so 32/64-bit builds do identical work.
 * Freestanding; prints markers + checksum. */
typedef unsigned int u32; typedef unsigned long ul;
#if defined(__x86_64__)
#define SW 1
#define SE 60
static inline long sc3(long n,long a,long b,long c){long r;register long r10 asm("r10")=0;asm volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(r10):"rcx","r11","memory");return r;}
#elif defined(__i386__)
#define SW 4
#define SE 1
static inline long sc3(long n,long a,long b,long c){long r;asm volatile("int $0x80":"=a"(r):"a"(n),"b"(a),"c"(b),"d"(c):"memory");return r;}
#elif defined(__riscv)
#define SW 64
#define SE 93
static inline long sc3(long n,long a,long b,long c){register long a7 asm("a7")=n,a0 asm("a0")=a,a1 asm("a1")=b,a2 asm("a2")=c;asm volatile("ecall":"+r"(a0):"r"(a7),"r"(a1),"r"(a2):"memory");return a0;}
#endif
#ifndef WORK
#define WORK 1
#endif
static void wr(const char*s,long n){sc3(SW,1,(long)s,n);}
static void wrhex(u32 v){char b[11];b[0]='0';b[1]='x';for(int i=0;i<8;i++){int x=(v>>((7-i)*4))&0xf;b[2+i]=x<10?'0'+x:'a'+x-10;}b[10]='\n';wr(b,11);}
void bench_main(void){
    wr("BENCH_START\n",12);
    u32 h=2463534242u, acc=0;
    for(long i=0;i<800000000L*WORK;i++){ h^=h<<13; h^=h>>17; h^=h<<5; acc+=h; }
    wr("checksum=",9); wrhex(acc); wr("BENCH_DONE\n",11);
    sc3(SE,0,0,0); for(;;){}
}
#if defined(__riscv)
__attribute__((naked)) void _start(void){__asm__ volatile(".option push\n.option norelax\nlla gp, __global_pointer$\n.option pop\nandi sp,sp,-16\ncall bench_main\n");}
#elif defined(__x86_64__)
__attribute__((naked)) void _start(void){__asm__ volatile("andq $-16,%rsp\ncall bench_main\n");}
#elif defined(__i386__)
__attribute__((naked)) void _start(void){__asm__ volatile("andl $-16,%esp\ncall bench_main\n");}
#endif
