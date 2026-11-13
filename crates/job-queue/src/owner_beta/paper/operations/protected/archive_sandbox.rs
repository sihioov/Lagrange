//! Additive process-wide restriction installed before reading protected inputs.
use super::{ErrorCode, Verb, bound::BoundOriginals};

const DAC_READ_SEARCH: u64 = 1 << 2;
const CAP_VERSION_3: u32 = 0x2008_0522;

#[repr(C)]
struct CapHeader {
    version: u32,
    pid: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapWord {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Capabilities {
    effective: u64,
    permitted: u64,
    inheritable: u64,
    ambient: u64,
}

fn allowed_caps(caps: Capabilities, expected: Option<u64>) -> bool {
    caps.inheritable == 0
        && caps.ambient == 0
        && caps.effective == caps.permitted
        && match expected {
            Some(value) => caps.effective == value,
            None => caps.effective == 0 || caps.effective == DAC_READ_SEARCH,
        }
}

fn required_capability(verb: Verb, purpose: Option<&str>, action: Option<Verb>) -> u64 {
    if verb == Verb::Begin && purpose == Some("RESTORE") && action == Some(Verb::Begin) {
        DAC_READ_SEARCH
    } else {
        0
    }
}

fn current_caps() -> Result<Capabilities, ErrorCode> {
    let mut header = CapHeader {
        version: CAP_VERSION_3,
        pid: 0,
    };
    let mut data = [CapWord::default(); 2];
    if unsafe { libc::syscall(libc::SYS_capget, &mut header, data.as_mut_ptr()) } != 0
        || header.version != CAP_VERSION_3
    {
        return Err(ErrorCode::ReaderIncompatible);
    }
    let combine = |lo: u32, hi: u32| u64::from(lo) | (u64::from(hi) << 32);
    let mut ambient = 0u64;
    let mut end = false;
    for cap in 0..=64u32 {
        // PR_CAP_AMBIENT / PR_CAP_AMBIENT_IS_SET. Reject unsupported APIs,
        // unexpected errors and capabilities beyond the capget v3 width.
        let result = unsafe { libc::prctl(47, 1, cap as libc::c_ulong, 0, 0) };
        if result == -1 {
            if cap > 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL) {
                end = true;
                break;
            }
            return Err(ErrorCode::ReaderIncompatible);
        }
        if cap == 64 || !(0..=1).contains(&result) {
            return Err(ErrorCode::ReaderIncompatible);
        }
        ambient |= (result as u64) << cap;
    }
    if !end {
        return Err(ErrorCode::ReaderIncompatible);
    }
    Ok(Capabilities {
        effective: combine(data[0].effective, data[1].effective),
        permitted: combine(data[0].permitted, data[1].permitted),
        inheritable: combine(data[0].inheritable, data[1].inheritable),
        ambient,
    })
}

#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Insn {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}
#[cfg(target_arch = "x86_64")]
#[repr(C)]
struct Program {
    len: u16,
    filter: *const Insn,
}

// This private witness is constructed only after the kernel accepts TSYNC.
// Dropping it cannot remove a seccomp filter.
pub(super) struct Guard {
    initial: Capabilities,
}

impl Guard {
    pub(super) fn revalidate(&self) -> Result<(), ErrorCode> {
        // These queries establish only process restrictions, not host isolation.
        if current_caps()? != self.initial
            || unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) } != 1
            || unsafe { libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) } != 2
        {
            return Err(ErrorCode::ReaderIncompatible);
        }
        Ok(())
    }

    pub(super) fn check_invocation(
        &self,
        verb: Verb,
        bound: Option<&BoundOriginals>,
    ) -> Result<(), ErrorCode> {
        self.revalidate()?;
        let expected = required_capability(
            verb,
            bound.map(|originals| originals.root.intent.purpose.as_str()),
            bound.map(|originals| originals.action().verb),
        );
        if !allowed_caps(self.initial, Some(expected)) {
            return Err(ErrorCode::ReaderIncompatible);
        }
        Ok(())
    }
}

#[cfg(target_arch = "x86_64")]
fn program() -> Vec<Insn> {
    const LD: u16 = 0x20;
    const JEQ: u16 = 0x15;
    const JGE: u16 = 0x35;
    const JSET: u16 = 0x45;
    const RET: u16 = 0x06;
    const KILL: u32 = 0x8000_0000;
    const ERRNO: u32 = 0x0005_0000;
    const ALLOW: u32 = 0x7fff_0000;
    let i = |code, jt, jf, k| Insn { code, jt, jf, k };
    let mut p = vec![
        i(LD, 0, 0, 4),
        i(JEQ, 1, 0, 0xc000_003e),
        i(RET, 0, 0, KILL),
        i(LD, 0, 0, 0),
        i(JSET, 0, 1, 0x4000_0000),
        i(RET, 0, 0, KILL),
        // Linux before 5.4 could confuse unmarked x32 syscall numbers.
        i(JGE, 0, 2, 512),
        i(JGE, 1, 0, 548),
        i(RET, 0, 0, KILL),
    ];
    // open_by_handle_at, name_to_handle_at, unshare, setns and mount API.
    for nr in [
        304, 303, 272, 308, 165, 166, 155, 430, 431, 432, 433, 428, 429, 442,
    ] {
        p.push(i(JEQ, 0, 1, nr));
        p.push(i(RET, 0, 0, ERRNO | libc::EPERM as u32));
    }
    // clone3 cannot be inspected safely; ENOSYS lets libc use clone for threads.
    p.push(i(JEQ, 0, 1, 435));
    p.push(i(RET, 0, 0, ERRNO | libc::ENOSYS as u32));
    // Legacy clone flags are its first argument, low 32 bits on x86-64.
    p.push(i(JEQ, 0, 3, 56));
    p.push(i(LD, 0, 0, 16));
    p.push(i(JSET, 0, 1, 0x7e02_0080));
    p.push(i(RET, 0, 0, ERRNO | libc::EPERM as u32));
    p.push(i(RET, 0, 0, ALLOW));
    p
}

// Testable installation sequence: no fallback after any failing kernel call.
fn install_sequence(mut call: impl FnMut(u8) -> i64) -> Result<(), ErrorCode> {
    if call(0) != 0 || call(1) != 1 || call(2) != 0 {
        return Err(ErrorCode::ReaderIncompatible);
    }
    Ok(())
}

pub(super) fn install() -> Result<Guard, ErrorCode> {
    #[cfg(target_arch = "x86_64")]
    {
        let initial = current_caps()?;
        if !allowed_caps(initial, None) {
            return Err(ErrorCode::ReaderIncompatible);
        }
        let filters = program();
        let descriptor = Program {
            len: filters.len() as u16,
            filter: filters.as_ptr(),
        };
        install_sequence(|step| unsafe {
            match step {
                0 => libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) as i64,
                1 => libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) as i64,
                _ => libc::syscall(libc::SYS_seccomp, 1u32, 1u32, &descriptor),
            }
        })?;
        let guard = Guard { initial };
        guard.revalidate()?;
        Ok(guard)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        Err(ErrorCode::ReaderIncompatible)
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;
    #[test]
    fn capability_policy_mutants() {
        let empty = Capabilities {
            effective: 0,
            permitted: 0,
            inheritable: 0,
            ambient: 0,
        };
        let dac = Capabilities {
            effective: DAC_READ_SEARCH,
            permitted: DAC_READ_SEARCH,
            ..empty
        };
        assert!(allowed_caps(empty, None));
        assert!(allowed_caps(dac, None));
        assert!(allowed_caps(empty, Some(0)));
        assert!(allowed_caps(dac, Some(DAC_READ_SEARCH)));
        assert!(!allowed_caps(dac, Some(0)));
        assert!(!allowed_caps(empty, Some(DAC_READ_SEARCH)));
        for bad in [
            Capabilities {
                effective: 1,
                permitted: 1,
                ..empty
            },
            Capabilities {
                effective: DAC_READ_SEARCH | 1,
                permitted: DAC_READ_SEARCH | 1,
                ..empty
            },
            Capabilities {
                effective: DAC_READ_SEARCH,
                ..empty
            },
            Capabilities {
                permitted: DAC_READ_SEARCH,
                ..empty
            },
            Capabilities {
                inheritable: DAC_READ_SEARCH,
                ..dac
            },
            Capabilities {
                ambient: DAC_READ_SEARCH,
                ..dac
            },
            Capabilities {
                effective: 1 << 40,
                permitted: 1 << 40,
                ..empty
            },
        ] {
            assert!(!allowed_caps(bad, None));
        }
        assert_eq!(
            required_capability(Verb::Begin, Some("RESTORE"), Some(Verb::Begin)),
            DAC_READ_SEARCH
        );
        for verb in [
            Verb::Configure,
            Verb::Inspect,
            Verb::Enable,
            Verb::Reconcile,
            Verb::Off,
            Verb::RestoreOff,
            Verb::Recover,
            Verb::Readback,
            Verb::ObserveReset,
        ] {
            assert_eq!(
                required_capability(verb, Some("RESTORE"), Some(Verb::Begin)),
                0
            );
        }
        for purpose in [None, Some("STOP"), Some("BACKUP"), Some("restore")] {
            assert_eq!(
                required_capability(Verb::Begin, purpose, Some(Verb::Begin)),
                0
            );
        }
        assert_eq!(required_capability(Verb::Begin, Some("RESTORE"), None), 0);
        assert_eq!(
            required_capability(Verb::Begin, Some("RESTORE"), Some(Verb::Off)),
            0
        );
    }
    fn evaluate(arch: u32, nr: u32, flags: u32) -> u32 {
        let p = program();
        let mut pc = 0usize;
        let mut a = 0;
        loop {
            let ins = p[pc];
            match ins.code {
                0x20 => {
                    a = match ins.k {
                        0 => nr,
                        4 => arch,
                        16 => flags,
                        _ => panic!("offset"),
                    };
                    pc += 1;
                }
                0x15 => pc += 1 + usize::from(if a == ins.k { ins.jt } else { ins.jf }),
                0x35 => pc += 1 + usize::from(if a >= ins.k { ins.jt } else { ins.jf }),
                0x45 => pc += 1 + usize::from(if a & ins.k != 0 { ins.jt } else { ins.jf }),
                0x06 => return ins.k,
                _ => panic!("opcode"),
            }
        }
    }
    #[test]
    fn filter_vectors() {
        let x = 0xc000_003e;
        assert_eq!(evaluate(0, 0, 0), 0x8000_0000);
        assert_eq!(evaluate(x, 0x4000_0000, 0), 0x8000_0000);
        for nr in [512, 521, 547] {
            assert_eq!(evaluate(x, nr, 0), 0x8000_0000);
        }
        for nr in [511, 548] {
            assert_eq!(evaluate(x, nr, 0), 0x7fff_0000);
        }
        for nr in [
            304, 303, 272, 308, 165, 166, 155, 430, 431, 432, 433, 428, 429, 442,
        ] {
            assert_eq!(evaluate(x, nr, 0), 0x0005_0000 | libc::EPERM as u32);
        }
        assert_eq!(evaluate(x, 435, 0), 0x0005_0000 | libc::ENOSYS as u32);
        for flag in [
            0x80, 0x20000, 0x02000000, 0x04000000, 0x08000000, 0x10000000, 0x20000000, 0x40000000,
        ] {
            assert_eq!(evaluate(x, 56, flag), 0x0005_0000 | libc::EPERM as u32);
        }
        assert_eq!(evaluate(x, 56, 0x10000), 0x7fff_0000);
        assert_eq!(evaluate(x, 0, 0), 0x7fff_0000);
    }
    #[test]
    fn install_requires_exact_results_and_never_retries() {
        for (failure, result) in [(0, -1), (1, 0), (2, -1), (2, 123)] {
            let mut calls = Vec::new();
            assert!(
                install_sequence(|step| {
                    calls.push(step);
                    if step == failure {
                        result
                    } else if step == 1 {
                        1
                    } else {
                        0
                    }
                })
                .is_err()
            );
            assert_eq!(calls, (0..=failure).collect::<Vec<_>>());
        }
        let mut calls = Vec::new();
        assert!(
            install_sequence(|step| {
                calls.push(step);
                if step == 1 { 1 } else { 0 }
            })
            .is_ok()
        );
        assert_eq!(calls, [0, 1, 2]);
    }

    // The test harness process never installs a filter. The dedicated child
    // verifies TSYNC on a preexisting thread and inheritance by a new thread.
    #[test]
    #[ignore = "requires hosted Linux kernel execution"]
    fn sandbox_kernel_child() {
        const CHILD: &str = "PAPER_ARCHIVE_SANDBOX_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            use std::io::Read;
            use std::process::Stdio;
            let mut child = std::process::Command::new("/proc/self/exe")
                .args(["--ignored", "sandbox_kernel_child"])
                .env(CHILD, "1")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            let out_send = send.clone();
            let out_thread = std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = stdout.take(8193).read_to_end(&mut bytes);
                out_send.send((0, result, bytes)).unwrap();
            });
            let err_thread = std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = stderr.take(8193).read_to_end(&mut bytes);
                send.send((1, result, bytes)).unwrap();
            });
            let mut streams = [Vec::new(), Vec::new()];
            let mut valid = true;
            for _ in 0..2 {
                let (which, result, bytes) = receive.recv().unwrap();
                if result.is_err() || bytes.len() > 8192 {
                    valid = false;
                    let _ = child.kill();
                }
                streams[which] = bytes;
            }
            let status = child.wait().unwrap();
            out_thread.join().unwrap();
            err_thread.join().unwrap();
            assert!(valid);
            assert!(status.success());
            let output = std::str::from_utf8(&streams[0]).unwrap();
            assert_eq!(
                output
                    .lines()
                    .filter(|line| line.starts_with("test ")
                        && line.contains("::sandbox_kernel_child ... ok"))
                    .count(),
                1
            );
            assert!(
                output
                    .lines()
                    .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;"))
            );
            return;
        }
        // Only the owned child drops capabilities; the harness parent is untouched.
        assert_eq!(unsafe { libc::prctl(47, 4, 0, 0, 0) }, 0); // AMBIENT_CLEAR_ALL
        let mut header = CapHeader {
            version: CAP_VERSION_3,
            pid: 0,
        };
        let data = [CapWord::default(); 2];
        assert_eq!(
            unsafe { libc::syscall(libc::SYS_capset, &mut header, data.as_ptr()) },
            0
        );
        assert!(allowed_caps(current_caps().unwrap(), Some(0)));
        fn handle_errno() -> Option<i32> {
            let result = unsafe {
                libc::syscall(
                    303,
                    -1i32,
                    std::ptr::null::<u8>(),
                    std::ptr::null::<u8>(),
                    std::ptr::null::<u8>(),
                    0i32,
                )
            };
            assert_eq!(result, -1);
            std::io::Error::last_os_error().raw_os_error()
        }
        let (send, receive) = std::sync::mpsc::channel();
        let (result_send, result_receive) = std::sync::mpsc::channel();
        let existing = std::thread::spawn(move || {
            result_send.send(handle_errno()).unwrap();
            receive.recv().unwrap();
            result_send.send(handle_errno()).unwrap();
        });
        // Without this baseline the daemon's own policy could mask a TSYNC failure.
        assert_ne!(handle_errno(), Some(libc::EPERM));
        assert_ne!(result_receive.recv().unwrap(), Some(libc::EPERM));
        let guard = install().unwrap();
        guard.revalidate().unwrap();
        assert_eq!(handle_errno(), Some(libc::EPERM));
        send.send(()).unwrap();
        assert_eq!(result_receive.recv().unwrap(), Some(libc::EPERM));
        existing.join().unwrap();
        assert_eq!(
            std::thread::spawn(handle_errno).join().unwrap(),
            Some(libc::EPERM)
        );
        let result = unsafe { libc::syscall(435, std::ptr::null::<u8>(), 0usize) };
        assert_eq!(result, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ENOSYS)
        );
    }
}
