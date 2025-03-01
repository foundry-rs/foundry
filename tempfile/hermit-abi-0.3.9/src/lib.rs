//! `hermit-abi` is small interface to call functions from the
//! [Hermit unikernel](https://github.com/hermit-os/kernel).

#![no_std]
#![allow(nonstandard_style)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::result_unit_err)]

pub mod errno;
pub mod tcplistener;
pub mod tcpstream;

pub use self::errno::*;
pub use core::ffi::{c_int, c_short, c_void};

/// A thread handle type
pub type Tid = u32;

/// Maximum number of priorities
pub const NO_PRIORITIES: usize = 31;

/// Priority of a thread
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct Priority(u8);

impl Priority {
	pub const fn into(self) -> u8 {
		self.0
	}

	pub const fn from(x: u8) -> Self {
		Priority(x)
	}
}

pub const HIGH_PRIO: Priority = Priority::from(3);
pub const NORMAL_PRIO: Priority = Priority::from(2);
pub const LOW_PRIO: Priority = Priority::from(1);

/// A handle, identifying a socket
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Handle(usize);

pub const NSEC_PER_SEC: u64 = 1_000_000_000;
pub const FUTEX_RELATIVE_TIMEOUT: u32 = 1;
pub const CLOCK_REALTIME: u64 = 1;
pub const CLOCK_MONOTONIC: u64 = 4;
pub const STDIN_FILENO: c_int = 0;
pub const STDOUT_FILENO: c_int = 1;
pub const STDERR_FILENO: c_int = 2;
pub const O_RDONLY: i32 = 0o0;
pub const O_WRONLY: i32 = 0o1;
pub const O_RDWR: i32 = 0o2;
pub const O_CREAT: i32 = 0o100;
pub const O_EXCL: i32 = 0o200;
pub const O_TRUNC: i32 = 0o1000;
pub const O_APPEND: i32 = 0o2000;
pub const O_NONBLOCK: i32 = 0o4000;
pub const F_DUPFD: i32 = 0;
pub const F_GETFD: i32 = 1;
pub const F_SETFD: i32 = 2;
pub const F_GETFL: i32 = 3;
pub const F_SETFL: i32 = 4;
pub const FD_CLOEXEC: i32 = 1;

/// returns true if file descriptor `fd` is a tty
pub fn isatty(_fd: c_int) -> bool {
	false
}

/// `timespec` is used by `clock_gettime` to retrieve the
/// current time
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct timespec {
	/// seconds
	pub tv_sec: i64,
	/// nanoseconds
	pub tv_nsec: i64,
}

/// Internet protocol version.
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum Version {
	Unspecified,
	Ipv4,
	Ipv6,
}

/// A four-octet IPv4 address.
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default)]
pub struct Ipv4Address(pub [u8; 4]);

/// A sixteen-octet IPv6 address.
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default)]
pub struct Ipv6Address(pub [u8; 16]);

/// An internetworking address.
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum IpAddress {
	/// An unspecified address.
	/// May be used as a placeholder for storage where the address is not assigned yet.
	Unspecified,
	/// An IPv4 address.
	Ipv4(Ipv4Address),
	/// An IPv6 address.
	Ipv6(Ipv6Address),
}

/// The largest number `rand` will return
pub const RAND_MAX: u64 = 2_147_483_647;

pub const AF_INET: i32 = 0;
pub const AF_INET6: i32 = 1;
pub const IPPROTO_IP: i32 = 0;
pub const IPPROTO_IPV6: i32 = 41;
pub const IPPROTO_UDP: i32 = 17;
pub const IPPROTO_TCP: i32 = 6;
pub const IPV6_ADD_MEMBERSHIP: i32 = 12;
pub const IPV6_DROP_MEMBERSHIP: i32 = 13;
pub const IPV6_MULTICAST_LOOP: i32 = 19;
pub const IPV6_V6ONLY: i32 = 27;
pub const IP_TOS: i32 = 1;
pub const IP_TTL: i32 = 2;
pub const IP_MULTICAST_TTL: i32 = 5;
pub const IP_MULTICAST_LOOP: i32 = 7;
pub const IP_ADD_MEMBERSHIP: i32 = 3;
pub const IP_DROP_MEMBERSHIP: i32 = 4;
pub const SHUT_RD: i32 = 0;
pub const SHUT_WR: i32 = 1;
pub const SHUT_RDWR: i32 = 2;
pub const SOCK_DGRAM: i32 = 2;
pub const SOCK_STREAM: i32 = 1;
pub const SOCK_NONBLOCK: i32 = 0o4000;
pub const SOCK_CLOEXEC: i32 = 0o40000;
pub const SOL_SOCKET: i32 = 4095;
pub const SO_REUSEADDR: i32 = 0x0004;
pub const SO_KEEPALIVE: i32 = 0x0008;
pub const SO_BROADCAST: i32 = 0x0020;
pub const SO_LINGER: i32 = 0x0080;
pub const SO_SNDBUF: i32 = 0x1001;
pub const SO_RCVBUF: i32 = 0x1002;
pub const SO_SNDTIMEO: i32 = 0x1005;
pub const SO_RCVTIMEO: i32 = 0x1006;
pub const SO_ERROR: i32 = 0x1007;
pub const TCP_NODELAY: i32 = 1;
pub const MSG_PEEK: i32 = 1;
pub const FIONBIO: i32 = 0x8008667eu32 as i32;
pub const EAI_NONAME: i32 = -2200;
pub const EAI_SERVICE: i32 = -2201;
pub const EAI_FAIL: i32 = -2202;
pub const EAI_MEMORY: i32 = -2203;
pub const EAI_FAMILY: i32 = -2204;
pub const POLLIN: i16 = 0x1;
pub const POLLPRI: i16 = 0x2;
pub const POLLOUT: i16 = 0x4;
pub const POLLERR: i16 = 0x8;
pub const POLLHUP: i16 = 0x10;
pub const POLLNVAL: i16 = 0x20;
pub const POLLRDNORM: i16 = 0x040;
pub const POLLRDBAND: i16 = 0x080;
pub const POLLWRNORM: i16 = 0x0100;
pub const POLLWRBAND: i16 = 0x0200;
pub const POLLRDHUP: i16 = 0x2000;
pub const EFD_SEMAPHORE: i16 = 0o1;
pub const EFD_NONBLOCK: i16 = 0o4000;
pub const EFD_CLOEXEC: i16 = 0o40000;
pub type sa_family_t = u8;
pub type socklen_t = u32;
pub type in_addr_t = u32;
pub type in_port_t = u16;
pub type time_t = i64;
pub type suseconds_t = i64;
pub type nfds_t = usize;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct in_addr {
	pub s_addr: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct in6_addr {
	pub s6_addr: [u8; 16],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct sockaddr {
	pub sa_len: u8,
	pub sa_family: sa_family_t,
	pub sa_data: [u8; 14],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct sockaddr_in {
	pub sin_len: u8,
	pub sin_family: sa_family_t,
	pub sin_port: u16,
	pub sin_addr: in_addr,
	pub sin_zero: [u8; 8],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct sockaddr_in6 {
	pub sin6_family: sa_family_t,
	pub sin6_port: u16,
	pub sin6_addr: in6_addr,
	pub sin6_flowinfo: u32,
	pub sin6_scope_id: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct addrinfo {
	pub ai_flags: i32,
	pub ai_family: i32,
	pub ai_socktype: i32,
	pub ai_protocol: i32,
	pub ai_addrlen: socklen_t,
	pub ai_addr: *mut sockaddr,
	pub ai_canonname: *mut u8,
	pub ai_next: *mut addrinfo,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct sockaddr_storage {
	pub s2_len: u8,
	pub ss_family: sa_family_t,
	pub s2_data1: [i8; 2usize],
	pub s2_data2: [u32; 3usize],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ip_mreq {
	pub imr_multiaddr: in_addr,
	pub imr_interface: in_addr,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ipv6_mreq {
	pub ipv6mr_multiaddr: in6_addr,
	pub ipv6mr_interface: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct linger {
	pub l_onoff: i32,
	pub l_linger: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct timeval {
	pub tv_sec: time_t,
	pub tv_usec: suseconds_t,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct pollfd {
	/// file descriptor
	pub fd: i32,
	/// events to look for
	pub events: i16,
	/// events returned
	pub revents: i16,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct stat {
	pub st_dev: u64,
	pub st_ino: u64,
	pub st_nlink: u64,
	/// access permissions
	pub st_mode: u32,
	/// user id
	pub st_uid: u32,
	/// group id
	pub st_gid: u32,
	/// device id
	pub st_rdev: u64,
	/// size in bytes
	pub st_size: u64,
	/// block size
	pub st_blksize: i64,
	/// size in blocks
	pub st_blocks: i64,
	/// time of last access
	pub st_atime: u64,
	pub st_atime_nsec: u64,
	/// time of last modification
	pub st_mtime: u64,
	pub st_mtime_nsec: u64,
	/// time of last status change
	pub st_ctime: u64,
	pub st_ctime_nsec: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct dirent64 {
	/// 64-bit inode number
	pub d_ino: u64,
	/// 64-bit offset to next structure
	pub d_off: i64,
	/// Size of this dirent
	pub d_reclen: u16,
	/// File type
	pub d_type: u8,
	/// Filename (null-terminated)
	pub d_name: core::marker::PhantomData<u8>,
}

pub const DT_UNKNOWN: u32 = 0;
pub const DT_FIFO: u32 = 1;
pub const DT_CHR: u32 = 2;
pub const DT_DIR: u32 = 4;
pub const DT_BLK: u32 = 6;
pub const DT_REG: u32 = 8;
pub const DT_LNK: u32 = 10;
pub const DT_SOCK: u32 = 12;
pub const DT_WHT: u32 = 14;

pub const S_IFDIR: u32 = 0x4000;
pub const S_IFREG: u32 = 0x8000;
pub const S_IFLNK: u32 = 0xA000;
pub const S_IFMT: u32 = 0xF000;

// sysmbols, which are part of the library operating system
extern "C" {
	/// Get the last error number from the thread local storage
	#[link_name = "sys_get_errno"]
	pub fn get_errno() -> i32;

	/// If the value at address matches the expected value, park the current thread until it is either
	/// woken up with [`futex_wake`] (returns 0) or an optional timeout elapses (returns -ETIMEDOUT).
	///
	/// Setting `timeout` to null means the function will only return if [`futex_wake`] is called.
	/// Otherwise, `timeout` is interpreted as an absolute time measured with [`CLOCK_MONOTONIC`].
	/// If [`FUTEX_RELATIVE_TIMEOUT`] is set in `flags` the timeout is understood to be relative
	/// to the current time.
	///
	/// Returns -EINVAL if `address` is null, the timeout is negative or `flags` contains unknown values.
	#[link_name = "sys_futex_wait"]
	pub fn futex_wait(
		address: *mut u32,
		expected: u32,
		timeout: *const timespec,
		flags: u32,
	) -> i32;

	/// Wake `count` threads waiting on the futex at `address`. Returns the number of threads
	/// woken up (saturates to `i32::MAX`). If `count` is `i32::MAX`, wake up all matching
	/// waiting threads. If `count` is negative or `address` is null, returns -EINVAL.
	#[link_name = "sys_futex_wake"]
	pub fn futex_wake(address: *mut u32, count: i32) -> i32;

	/// sem_init() initializes the unnamed semaphore at the address
	/// pointed to by `sem`.  The `value` argument specifies the
	/// initial value for the semaphore.
	#[link_name = "sys_sem_init"]
	pub fn sem_init(sem: *mut *const c_void, value: u32) -> i32;

	/// sem_destroy() frees the unnamed semaphore at the address
	/// pointed to by `sem`.
	#[link_name = "sys_sem_destroy"]
	pub fn sem_destroy(sem: *const c_void) -> i32;

	/// sem_post() increments the semaphore pointed to by `sem`.
	/// If the semaphore's value consequently becomes greater
	/// than zero, then another thread blocked in a sem_wait call
	/// will be woken up and proceed to lock the semaphore.
	#[link_name = "sys_sem_post"]
	pub fn sem_post(sem: *const c_void) -> i32;

	/// try to decrement a semaphore
	///
	/// sem_trywait() is the same as sem_timedwait(), except that
	/// if the  decrement cannot be immediately performed, then  call
	/// returns a negative value instead of blocking.
	#[link_name = "sys_sem_trywait"]
	pub fn sem_trywait(sem: *const c_void) -> i32;

	/// decrement a semaphore
	///
	/// sem_timedwait() decrements the semaphore pointed to by `sem`.
	/// If the semaphore's value is greater than zero, then the
	/// the function returns immediately. If the semaphore currently
	/// has the value zero, then the call blocks until either
	/// it becomes possible to perform the decrement of the time limit
	/// to wait for the semaphore is expired. A time limit `ms` of
	/// means infinity waiting time.
	#[link_name = "sys_timedwait"]
	pub fn sem_timedwait(sem: *const c_void, ms: u32) -> i32;

	/// Determines the id of the current thread
	#[link_name = "sys_getpid"]
	pub fn getpid() -> u32;

	/// cause normal termination and return `arg`
	/// to the host system
	#[link_name = "sys_exit"]
	pub fn exit(arg: i32) -> !;

	/// cause abnormal termination
	#[link_name = "sys_abort"]
	pub fn abort() -> !;

	/// suspend execution for microsecond intervals
	///
	/// The usleep() function suspends execution of the calling
	/// thread for (at least) `usecs` microseconds.
	#[link_name = "sys_usleep"]
	pub fn usleep(usecs: u64);

	/// spawn a new thread
	///
	/// spawn() starts a new thread. The new thread starts execution
	/// by invoking `func(usize)`; `arg` is passed as the argument
	/// to `func`. `prio` defines the priority of the new thread,
	/// which can be between `LOW_PRIO` and `HIGH_PRIO`.
	/// `core_id` defines the core, where the thread is located.
	/// A negative value give the operating system the possibility
	/// to select the core by its own.
	#[link_name = "sys_spawn"]
	pub fn spawn(
		id: *mut Tid,
		func: extern "C" fn(usize),
		arg: usize,
		prio: u8,
		core_id: isize,
	) -> i32;

	/// spawn a new thread with user-specified stack size
	///
	/// spawn2() starts a new thread. The new thread starts execution
	/// by invoking `func(usize)`; `arg` is passed as the argument
	/// to `func`. `prio` defines the priority of the new thread,
	/// which can be between `LOW_PRIO` and `HIGH_PRIO`.
	/// `core_id` defines the core, where the thread is located.
	/// A negative value give the operating system the possibility
	/// to select the core by its own.
	/// In contrast to spawn(), spawn2() is able to define the
	/// stack size.
	#[link_name = "sys_spawn2"]
	pub fn spawn2(
		func: extern "C" fn(usize),
		arg: usize,
		prio: u8,
		stack_size: usize,
		core_id: isize,
	) -> Tid;

	/// join with a terminated thread
	///
	/// The join() function waits for the thread specified by `id`
	/// to terminate.
	#[link_name = "sys_join"]
	pub fn join(id: Tid) -> i32;

	/// yield the processor
	///
	/// causes the calling thread to relinquish the CPU. The thread
	/// is moved to the end of the queue for its static priority.
	#[link_name = "sys_yield"]
	pub fn yield_now();

	/// get current time
	///
	/// The clock_gettime() functions allow the calling thread
	/// to retrieve the value used by a clock which is specified
	/// by `clock_id`.
	///
	/// `CLOCK_REALTIME`: the system's real time clock,
	/// expressed as the amount of time since the Epoch.
	///
	/// `CLOCK_MONOTONIC`: clock that increments monotonically,
	/// tracking the time since an arbitrary point
	#[link_name = "sys_clock_gettime"]
	pub fn clock_gettime(clock_id: u64, tp: *mut timespec) -> i32;

	/// open and possibly create a file
	///
	/// The open() system call opens the file specified by `name`.
	/// If the specified file does not exist, it may optionally
	/// be created by open().
	#[link_name = "sys_open"]
	pub fn open(name: *const i8, flags: i32, mode: i32) -> i32;

	/// open a directory
	///
	/// The opendir() system call opens the directory specified by `name`.
	#[link_name = "sys_opendir"]
	pub fn opendir(name: *const i8) -> i32;

	/// delete the file it refers to `name`
	#[link_name = "sys_unlink"]
	pub fn unlink(name: *const i8) -> i32;

	/// remove directory it refers to `name`
	#[link_name = "sys_rmdir"]
	pub fn rmdir(name: *const i8) -> i32;

	/// stat
	#[link_name = "sys_stat"]
	pub fn stat(name: *const i8, stat: *mut stat) -> i32;

	/// lstat
	#[link_name = "sys_lstat"]
	pub fn lstat(name: *const i8, stat: *mut stat) -> i32;

	/// fstat
	#[link_name = "sys_fstat"]
	pub fn fstat(fd: i32, stat: *mut stat) -> i32;

	/// determines the number of activated processors
	#[link_name = "sys_get_processor_count"]
	pub fn get_processor_count() -> usize;

	#[link_name = "sys_malloc"]
	pub fn malloc(size: usize, align: usize) -> *mut u8;

	#[doc(hidden)]
	#[link_name = "sys_realloc"]
	pub fn realloc(ptr: *mut u8, size: usize, align: usize, new_size: usize) -> *mut u8;

	#[doc(hidden)]
	#[link_name = "sys_free"]
	pub fn free(ptr: *mut u8, size: usize, align: usize);

	#[link_name = "sys_notify"]
	pub fn notify(id: usize, count: i32) -> i32;

	#[doc(hidden)]
	#[link_name = "sys_add_queue"]
	pub fn add_queue(id: usize, timeout_ns: i64) -> i32;

	#[doc(hidden)]
	#[link_name = "sys_wait"]
	pub fn wait(id: usize) -> i32;

	#[doc(hidden)]
	#[link_name = "sys_init_queue"]
	pub fn init_queue(id: usize) -> i32;

	#[doc(hidden)]
	#[link_name = "sys_destroy_queue"]
	pub fn destroy_queue(id: usize) -> i32;

	/// initialize the network stack
	#[link_name = "sys_network_init"]
	pub fn network_init() -> i32;

	/// Add current task to the queue of blocked tasks. After calling `block_current_task`,
	/// call `yield_now` to switch to another task.
	#[link_name = "sys_block_current_task"]
	pub fn block_current_task();

	/// Add current task to the queue of blocked tasks, but wake it when `timeout` milliseconds
	/// have elapsed.
	///
	/// After calling `block_current_task`, call `yield_now` to switch to another task.
	#[link_name = "sys_block_current_task_with_timeout"]
	pub fn block_current_task_with_timeout(timeout: u64);

	/// Wakeup task with the thread id `tid`
	#[link_name = "sys_wakeup_taskt"]
	pub fn wakeup_task(tid: Tid);

	#[link_name = "sys_accept"]
	pub fn accept(s: i32, addr: *mut sockaddr, addrlen: *mut socklen_t) -> i32;

	/// bind a name to a socket
	#[link_name = "sys_bind"]
	pub fn bind(s: i32, name: *const sockaddr, namelen: socklen_t) -> i32;

	#[link_name = "sys_connect"]
	pub fn connect(s: i32, name: *const sockaddr, namelen: socklen_t) -> i32;

	/// read from a file descriptor
	///
	/// read() attempts to read `len` bytes of data from the object
	/// referenced by the descriptor `fd` into the buffer pointed
	/// to by `buf`.
	#[link_name = "sys_read"]
	pub fn read(fd: i32, buf: *mut u8, len: usize) -> isize;

	/// `getdents64` reads directory entries from the directory referenced
	/// by the file descriptor `fd` into the buffer pointed to by `buf`.
	#[link_name = "sys_getdents64"]
	pub fn getdents64(fd: i32, dirp: *mut dirent64, count: usize) -> i64;

	/// 'mkdir' attempts to create a directory,
	/// it returns 0 on success and -1 on error
	#[link_name = "sys_mkdir"]
	pub fn mkdir(name: *const i8, mode: u32) -> i32;

	/// Fill `len` bytes in `buf` with cryptographically secure random data.
	///
	/// Returns either the number of bytes written to buf (a positive value) or
	/// * `-EINVAL` if `flags` contains unknown flags.
	/// * `-ENOSYS` if the system does not support random data generation.
	#[link_name = "sys_read_entropy"]
	pub fn read_entropy(buf: *mut u8, len: usize, flags: u32) -> isize;

	/// receive() a message from a socket
	#[link_name = "sys_recv"]
	pub fn recv(socket: i32, buf: *mut u8, len: usize, flags: i32) -> isize;

	/// receive() a message from a socket
	#[link_name = "sys_recvfrom"]
	pub fn recvfrom(
		socket: i32,
		buf: *mut u8,
		len: usize,
		flags: i32,
		addr: *mut sockaddr,
		addrlen: *mut socklen_t,
	) -> isize;

	/// write to a file descriptor
	///
	/// write() attempts to write `len` of data to the object
	/// referenced by the descriptor `fd` from the
	/// buffer pointed to by `buf`.
	#[link_name = "sys_write"]
	pub fn write(fd: i32, buf: *const u8, len: usize) -> isize;

	/// close a file descriptor
	///
	/// The close() call deletes a file descriptor `fd` from the object
	/// reference table.
	#[link_name = "sys_close"]
	pub fn close(fd: i32) -> i32;

	/// duplicate an existing file descriptor
	#[link_name = "sys_dup"]
	pub fn dup(fd: i32) -> i32;

	#[link_name = "sys_getpeername"]
	pub fn getpeername(s: i32, name: *mut sockaddr, namelen: *mut socklen_t) -> i32;

	#[link_name = "sys_getsockname"]
	pub fn getsockname(s: i32, name: *mut sockaddr, namelen: *mut socklen_t) -> i32;

	#[link_name = "sys_getsockopt"]
	pub fn getsockopt(
		s: i32,
		level: i32,
		optname: i32,
		optval: *mut c_void,
		optlen: *mut socklen_t,
	) -> i32;

	#[link_name = "sys_setsockopt"]
	pub fn setsockopt(
		s: i32,
		level: i32,
		optname: i32,
		optval: *const c_void,
		optlen: socklen_t,
	) -> i32;

	#[link_name = "sys_ioctl"]
	pub fn ioctl(s: i32, cmd: i32, argp: *mut c_void) -> i32;

	#[link_name = "sys_fcntl"]
	pub fn fcntl(fd: i32, cmd: i32, arg: i32) -> i32;

	/// `eventfd` creates an linux-like "eventfd object" that can be used
	/// as an event wait/notify mechanism by user-space applications, and by
	/// the kernel to notify user-space applications of events. The
	/// object contains an unsigned 64-bit integer counter
	/// that is maintained by the kernel. This counter is initialized
	/// with the value specified in the argument `initval`.
	///
	/// As its return value, `eventfd` returns a new file descriptor that
	/// can be used to refer to the eventfd object.
	///
	/// The following values may be bitwise set in flags to change the
	/// behavior of `eventfd`:
	///
	/// `EFD_NONBLOCK`: Set the file descriptor in non-blocking mode
	/// `EFD_SEMAPHORE`: Provide semaphore-like semantics for reads
	/// from the new file descriptor.
	#[link_name = "sys_eventfd"]
	pub fn eventfd(initval: u64, flags: i16) -> i32;

	/// The unix-like `poll` waits for one of a set of file descriptors
	/// to become ready to perform I/O. The set of file descriptors to be
	/// monitored is specified in the `fds` argument, which is an array
	/// of structures of `pollfd`.
	#[link_name = "sys_poll"]
	pub fn poll(fds: *mut pollfd, nfds: nfds_t, timeout: i32) -> i32;

	/// listen for connections on a socket
	///
	/// The `backlog` parameter defines the maximum length for the queue of pending
	/// connections. Currently, the `backlog` must be one.
	#[link_name = "sys_listen"]
	pub fn listen(s: i32, backlog: i32) -> i32;

	#[link_name = "sys_send"]
	pub fn send(s: i32, mem: *const c_void, len: usize, flags: i32) -> isize;

	#[link_name = "sys_sendto"]
	pub fn sendto(
		s: i32,
		mem: *const c_void,
		len: usize,
		flags: i32,
		to: *const sockaddr,
		tolen: socklen_t,
	) -> isize;

	/// shut down part of a full-duplex connection
	#[link_name = "sys_shutdown_socket"]
	pub fn shutdown_socket(s: i32, how: i32) -> i32;

	#[link_name = "sys_socket"]
	pub fn socket(domain: i32, type_: i32, protocol: i32) -> i32;

	#[link_name = "sys_freeaddrinfo"]
	pub fn freeaddrinfo(ai: *mut addrinfo);

	#[link_name = "sys_getaddrinfo"]
	pub fn getaddrinfo(
		nodename: *const i8,
		servname: *const u8,
		hints: *const addrinfo,
		res: *mut *mut addrinfo,
	) -> i32;

	fn sys_get_priority() -> u8;
	fn sys_set_priority(tid: Tid, prio: u8);
}

/// Determine the priority of the current thread
#[inline(always)]
pub unsafe fn get_priority() -> Priority {
	Priority::from(sys_get_priority())
}

/// Determine the priority of the current thread
#[inline(always)]
pub unsafe fn set_priority(tid: Tid, prio: Priority) {
	sys_set_priority(tid, prio.into());
}
