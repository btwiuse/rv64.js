import sys


def fib(n):
    return n if n < 2 else fib(n - 2) + fib(n - 1)


print("FIB_START", flush=True)
print("fib(30)=", fib(30), flush=True)
print("FIB_DONE", flush=True)
