#define _GNU_SOURCE
#include <sys/epoll.h>
#include <sys/syscall.h>
#include <unistd.h>
/* PostgreSQL's SIGALRM handler must see the same shifted wall clock as its query.
 * Linux epoll_wait itself needs no wall-clock translation for this offset-only fixture. */
int epoll_wait(int fd, struct epoll_event *events, int maximum, int timeout) {
    return (int) syscall(SYS_epoll_wait, fd, events, maximum, timeout);
}
