#include <netdb.h>

int res_query(const char *name, int class, int type,
              unsigned char *dest, int len)
{
    h_errno = HOST_NOT_FOUND;
    return -1;
}
