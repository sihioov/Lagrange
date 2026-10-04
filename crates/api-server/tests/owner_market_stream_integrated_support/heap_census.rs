//! Test-only global-allocator block census for the reviewed Linux GNU target.
//! Native allocations, explicit System bypasses and fragmentation are excluded.
//! Usable-byte peaks conservatively include old/new overlap during realloc.

#[cfg(not(all(target_os = "linux", target_env = "gnu", target_arch = "x86_64")))]
compile_error!("the usable-block census is restricted to reviewed x86_64 Linux GNU");

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

unsafe extern "C" {
    fn malloc_usable_size(pointer: *mut c_void) -> usize;
}

/// Returned allocations must be owned by the same glibc malloc family as
/// malloc_usable_size. Methods must not recurse through the global allocator,
/// acquire the census lock, unwind, or invoke user callbacks.
///
/// # Safety
/// All successful pointers, including over-aligned and realloc results, must
/// satisfy that allocator-family contract until deallocation/reallocation.
pub unsafe trait MallocCompatible: GlobalAlloc {}

// SAFETY: reviewed exact Linux GNU System implementation delegates to malloc,
// calloc, realloc, free, and posix_memalign. This is not a portable assumption.
unsafe impl MallocCompatible for System {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub current_usable_bytes: u64,
    pub conservative_peak_usable_bytes: u64,
    pub live_blocks: u64,
    pub allocations: u64,
    pub reallocations: u64,
    pub deallocations: u64,
    pub failed_allocations: u64,
    pub failed_reallocations: u64,
    pub diagnostic_error: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSnapshot {
    pub generation: u64,
    pub baseline_current_usable_bytes: u64,
    pub peak_usable_bytes: u64,
    pub current_usable_bytes: u64,
    pub diagnostic_error: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowError {
    AlreadyActive,
    NotCurrent,
    CounterExhausted,
}

struct State {
    totals: Snapshot,
    next_generation: u64,
    active_generation: Option<u64>,
    baseline: u64,
    peak: u64,
}

impl State {
    const fn new() -> Self {
        Self {
            totals: Snapshot {
                current_usable_bytes: 0,
                conservative_peak_usable_bytes: 0,
                live_blocks: 0,
                allocations: 0,
                reallocations: 0,
                deallocations: 0,
                failed_allocations: 0,
                failed_reallocations: 0,
                diagnostic_error: false,
            },
            next_generation: 1,
            active_generation: None,
            baseline: 0,
            peak: 0,
        }
    }

    fn increase(value: &mut u64, amount: u64, error: &mut bool) {
        match value.checked_add(amount) {
            Some(next) => *value = next,
            None => {
                *value = u64::MAX;
                *error = true;
            }
        }
    }

    fn decrease(value: &mut u64, amount: u64, error: &mut bool) {
        match value.checked_sub(amount) {
            Some(next) => *value = next,
            None => {
                *value = 0;
                *error = true;
            }
        }
    }

    fn observe_peak(&mut self, bytes: u64) {
        self.totals.conservative_peak_usable_bytes =
            self.totals.conservative_peak_usable_bytes.max(bytes);
        if self.active_generation.is_some() {
            self.peak = self.peak.max(bytes);
        }
    }

    fn allocated(&mut self, usable: u64, requested: usize) {
        if usable < requested as u64 {
            self.totals.diagnostic_error = true;
        }
        State::increase(
            &mut self.totals.allocations,
            1,
            &mut self.totals.diagnostic_error,
        );
        State::increase(
            &mut self.totals.live_blocks,
            1,
            &mut self.totals.diagnostic_error,
        );
        State::increase(
            &mut self.totals.current_usable_bytes,
            usable,
            &mut self.totals.diagnostic_error,
        );
        self.observe_peak(self.totals.current_usable_bytes);
    }

    fn freed(&mut self, usable: u64) {
        State::increase(
            &mut self.totals.deallocations,
            1,
            &mut self.totals.diagnostic_error,
        );
        State::decrease(
            &mut self.totals.live_blocks,
            1,
            &mut self.totals.diagnostic_error,
        );
        State::decrease(
            &mut self.totals.current_usable_bytes,
            usable,
            &mut self.totals.diagnostic_error,
        );
    }

    fn reallocated(&mut self, old: u64, new: u64, requested: usize) {
        if new < requested as u64 {
            self.totals.diagnostic_error = true;
        }
        State::increase(
            &mut self.totals.reallocations,
            1,
            &mut self.totals.diagnostic_error,
        );
        // System's aligned realloc can allocate-copy-free. Count old + new
        // conservatively, including when libc happened to resize in place.
        let mut overlap = self.totals.current_usable_bytes;
        State::increase(&mut overlap, new, &mut self.totals.diagnostic_error);
        self.observe_peak(overlap);
        State::decrease(
            &mut self.totals.current_usable_bytes,
            old,
            &mut self.totals.diagnostic_error,
        );
        State::increase(
            &mut self.totals.current_usable_bytes,
            new,
            &mut self.totals.diagnostic_error,
        );
        self.observe_peak(self.totals.current_usable_bytes);
    }
}

pub struct Census<A: MallocCompatible = System> {
    inner: A,
    locked: AtomicBool,
    state: UnsafeCell<State>,
}

// SAFETY: all access to UnsafeCell is under the fixed spin lock. Inner allocator
// access also stays under that lock; A meets the malloc-compatible contract.
unsafe impl<A: MallocCompatible + Send> Sync for Census<A> {}

struct Locked<'a, A: MallocCompatible> {
    owner: &'a Census<A>,
}

impl<A: MallocCompatible> Locked<'_, A> {
    fn state(&mut self) -> &mut State {
        // SAFETY: this guard exclusively owns the lock until Drop.
        unsafe { &mut *self.owner.state.get() }
    }
}

impl<A: MallocCompatible> Drop for Locked<'_, A> {
    fn drop(&mut self) {
        self.owner.locked.store(false, Ordering::Release);
    }
}

impl<A: MallocCompatible> Census<A> {
    pub const fn new(inner: A) -> Self {
        Self {
            inner,
            locked: AtomicBool::new(false),
            state: UnsafeCell::new(State::new()),
        }
    }

    fn lock(&self) -> Locked<'_, A> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        Locked { owner: self }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.lock().state().totals
    }

    pub fn begin_window(&self) -> Result<Window<'_, A>, WindowError> {
        let mut locked = self.lock();
        let state = locked.state();
        if state.active_generation.is_some() {
            return Err(WindowError::AlreadyActive);
        }
        let generation = state.next_generation;
        let Some(next) = generation.checked_add(1) else {
            state.totals.diagnostic_error = true;
            return Err(WindowError::CounterExhausted);
        };
        state.next_generation = next;
        state.active_generation = Some(generation);
        state.baseline = state.totals.current_usable_bytes;
        state.peak = state.baseline;
        Ok(Window {
            owner: self,
            generation,
            active: true,
        })
    }
}

/// Window identity never resets outstanding allocations or lifetime counters.
/// Drop cancels an unfinished observation and latches a diagnostic failure.
pub struct Window<'a, A: MallocCompatible> {
    owner: &'a Census<A>,
    generation: u64,
    active: bool,
}

impl<A: MallocCompatible> Window<'_, A> {
    pub fn snapshot(&self) -> Result<WindowSnapshot, WindowError> {
        let mut locked = self.owner.lock();
        let state = locked.state();
        if !self.active || state.active_generation != Some(self.generation) {
            return Err(WindowError::NotCurrent);
        }
        Ok(WindowSnapshot {
            generation: self.generation,
            baseline_current_usable_bytes: state.baseline,
            peak_usable_bytes: state.peak,
            current_usable_bytes: state.totals.current_usable_bytes,
            diagnostic_error: state.totals.diagnostic_error,
        })
    }

    pub fn finish(mut self) -> Result<WindowSnapshot, WindowError> {
        let mut locked = self.owner.lock();
        let state = locked.state();
        if !self.active || state.active_generation != Some(self.generation) {
            return Err(WindowError::NotCurrent);
        }
        let result = WindowSnapshot {
            generation: self.generation,
            baseline_current_usable_bytes: state.baseline,
            peak_usable_bytes: state.peak,
            current_usable_bytes: state.totals.current_usable_bytes,
            diagnostic_error: state.totals.diagnostic_error,
        };
        state.active_generation = None;
        self.active = false;
        Ok(result)
    }
}

impl<A: MallocCompatible> Drop for Window<'_, A> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut locked = self.owner.lock();
        let state = locked.state();
        if state.active_generation == Some(self.generation) {
            state.active_generation = None;
            state.totals.diagnostic_error = true;
        }
    }
}

// SAFETY: callers retain all GlobalAlloc Layout/pointer obligations. The inner
// allocator receives unchanged arguments. The lock, arithmetic, and libc usable
// size query do not allocate or unwind; no pointer bookkeeping is stored.
unsafe impl<A: MallocCompatible> GlobalAlloc for Census<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut locked = self.lock();
        let pointer = unsafe { self.inner.alloc(layout) };
        if pointer.is_null() {
            let state = locked.state();
            State::increase(
                &mut state.totals.failed_allocations,
                1,
                &mut state.totals.diagnostic_error,
            );
        } else {
            let usable = unsafe { malloc_usable_size(pointer.cast()) } as u64;
            locked.state().allocated(usable, layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let mut locked = self.lock();
        let pointer = unsafe { self.inner.alloc_zeroed(layout) };
        if pointer.is_null() {
            let state = locked.state();
            State::increase(
                &mut state.totals.failed_allocations,
                1,
                &mut state.totals.diagnostic_error,
            );
        } else {
            let usable = unsafe { malloc_usable_size(pointer.cast()) } as u64;
            locked.state().allocated(usable, layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let mut locked = self.lock();
        let usable = unsafe { malloc_usable_size(pointer.cast()) } as u64;
        unsafe { self.inner.dealloc(pointer, layout) };
        locked.state().freed(usable);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let mut locked = self.lock();
        let old_usable = unsafe { malloc_usable_size(pointer.cast()) } as u64;
        let new_pointer = unsafe { self.inner.realloc(pointer, layout, new_size) };
        if new_pointer.is_null() {
            let state = locked.state();
            State::increase(
                &mut state.totals.failed_reallocations,
                1,
                &mut state.totals.diagnostic_error,
            );
        } else {
            let new_usable = unsafe { malloc_usable_size(new_pointer.cast()) } as u64;
            locked.state().reallocated(old_usable, new_usable, new_size);
        }
        new_pointer
    }
}

#[cfg(test)]
mod arithmetic_tests {
    use super::*;

    #[test]
    fn counter_overflow_latches_without_wrapping() {
        let mut state = State::new();
        state.totals.current_usable_bytes = u64::MAX - 1;
        state.totals.allocations = u64::MAX;
        state.allocated(8, 1);
        assert_eq!(state.totals.current_usable_bytes, u64::MAX);
        assert_eq!(state.totals.allocations, u64::MAX);
        assert!(state.totals.diagnostic_error);
    }

    #[test]
    fn underflow_and_short_usable_size_latch_diagnostics() {
        let mut state = State::new();
        state.allocated(4, 8);
        assert!(state.totals.diagnostic_error);
        state.freed(16);
        assert_eq!(state.totals.current_usable_bytes, 0);
        assert!(state.totals.diagnostic_error);
        state.freed(0);
        assert_eq!(state.totals.live_blocks, 0);
        assert!(state.totals.diagnostic_error);
    }

    #[test]
    fn window_generation_exhaustion_fails_closed() {
        let allocator = Census::new(System);
        allocator.lock().state().next_generation = u64::MAX;
        assert!(matches!(
            allocator.begin_window(),
            Err(WindowError::CounterExhausted)
        ));
        assert!(allocator.snapshot().diagnostic_error);
    }
}
