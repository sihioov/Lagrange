#[cfg(test)]
mod tests {
    use crate::TEST_HEAP as GLOBAL;
    use crate::heap_census as census;
    use census::{Census, MallocCompatible, WindowError};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::Barrier;

    #[test]
    fn actual_allocations_balance_and_report_usable_blocks() {
        let allocator = Census::new(System);
        for size in [1, 17, 1024, 65536] {
            let layout = Layout::from_size_align(size, 8).unwrap();
            let pointer = unsafe { allocator.alloc(layout) };
            assert!(!pointer.is_null());
            unsafe {
                pointer.write_bytes(0x5a, size);
            }
            assert_eq!(unsafe { *pointer.add(size - 1) }, 0x5a);
            let live = allocator.snapshot();
            assert_eq!(live.live_blocks, 1);
            assert!(live.current_usable_bytes >= size as u64);
            unsafe {
                allocator.dealloc(pointer, layout);
            }
            assert_eq!(allocator.snapshot().current_usable_bytes, 0);
        }
        let final_value = allocator.snapshot();
        assert_eq!(
            (
                final_value.allocations,
                final_value.deallocations,
                final_value.live_blocks
            ),
            (4, 4, 0)
        );
        assert!(!final_value.diagnostic_error);
    }

    #[test]
    fn zeroed_overaligned_grow_and_shrink_preserve_payload() {
        let allocator = Census::new(System);
        for alignment in [8, 64, 4096, 65536] {
            let layout = Layout::from_size_align(137, alignment).unwrap();
            let pointer = unsafe { allocator.alloc_zeroed(layout) };
            assert!(!pointer.is_null());
            assert_eq!(pointer as usize % alignment, 0);
            assert!(
                unsafe { std::slice::from_raw_parts(pointer, 137) }
                    .iter()
                    .all(|b| *b == 0)
            );
            unsafe {
                pointer.write_bytes(0x31, 137);
            }
            let grown = unsafe { allocator.realloc(pointer, layout, 8193) };
            assert!(!grown.is_null());
            assert_eq!(grown as usize % alignment, 0);
            assert!(
                unsafe { std::slice::from_raw_parts(grown, 137) }
                    .iter()
                    .all(|b| *b == 0x31)
            );
            let large = Layout::from_size_align(8193, alignment).unwrap();
            let shrunk = unsafe { allocator.realloc(grown, large, 32) };
            assert!(!shrunk.is_null());
            assert_eq!(shrunk as usize % alignment, 0);
            assert!(
                unsafe { std::slice::from_raw_parts(shrunk, 32) }
                    .iter()
                    .all(|b| *b == 0x31)
            );
            unsafe {
                allocator.dealloc(shrunk, Layout::from_size_align(32, alignment).unwrap());
            }
        }
        let final_value = allocator.snapshot();
        assert_eq!(
            (
                final_value.allocations,
                final_value.reallocations,
                final_value.deallocations
            ),
            (4, 8, 4)
        );
        assert_eq!(final_value.current_usable_bytes, 0);
        assert!(!final_value.diagnostic_error);
    }

    struct FailedRealloc;
    unsafe impl GlobalAlloc for FailedRealloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) }
        }
        unsafe fn realloc(&self, _pointer: *mut u8, _layout: Layout, _new_size: usize) -> *mut u8 {
            std::ptr::null_mut()
        }
    }
    // SAFETY: successful pointers use System; failed realloc leaves ownership intact.
    unsafe impl MallocCompatible for FailedRealloc {}

    #[test]
    fn failed_realloc_keeps_original_block_and_accounting() {
        let allocator = Census::new(FailedRealloc);
        let layout = Layout::from_size_align(128, 16).unwrap();
        let pointer = unsafe { allocator.alloc(layout) };
        assert!(!pointer.is_null());
        unsafe {
            pointer.write_bytes(0x7b, 128);
        }
        let before = allocator.snapshot();
        let failed = unsafe { allocator.realloc(pointer, layout, 256) };
        assert!(failed.is_null());
        assert!(
            unsafe { std::slice::from_raw_parts(pointer, 128) }
                .iter()
                .all(|b| *b == 0x7b)
        );
        let after = allocator.snapshot();
        assert_eq!(after.current_usable_bytes, before.current_usable_bytes);
        assert_eq!(after.live_blocks, 1);
        assert_eq!((after.failed_reallocations, after.reallocations), (1, 0));
        unsafe {
            allocator.dealloc(pointer, layout);
        }
        assert_eq!(allocator.snapshot().current_usable_bytes, 0);
        assert!(!allocator.snapshot().diagnostic_error);
    }

    struct FailedAlloc;
    unsafe impl GlobalAlloc for FailedAlloc {
        unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
            std::ptr::null_mut()
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) }
        }
    }
    // SAFETY: this test allocator never issues a pointer; inherited zeroed handles null.
    unsafe impl MallocCompatible for FailedAlloc {}

    #[test]
    fn allocation_failure_is_counted_without_false_live_bytes() {
        let allocator = Census::new(FailedAlloc);
        let layout = Layout::from_size_align(64, 8).unwrap();
        assert!(unsafe { allocator.alloc(layout) }.is_null());
        assert!(unsafe { allocator.alloc_zeroed(layout) }.is_null());
        let value = allocator.snapshot();
        assert_eq!(
            (
                value.failed_allocations,
                value.allocations,
                value.live_blocks,
                value.current_usable_bytes
            ),
            (2, 0, 0, 0)
        );
        assert!(!value.diagnostic_error);
    }

    #[test]
    fn cross_thread_free_balances_the_same_census() {
        let allocator = Census::new(System);
        let layout = Layout::from_size_align(1024, 64).unwrap();
        let pointer = unsafe { allocator.alloc(layout) };
        assert!(!pointer.is_null());
        unsafe {
            pointer.write(91);
        }
        let address = pointer as usize;
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let pointer = address as *mut u8;
                    assert_eq!(unsafe { *pointer }, 91);
                    unsafe {
                        allocator.dealloc(pointer, layout);
                    }
                })
                .join()
                .unwrap();
        });
        let value = allocator.snapshot();
        assert_eq!((value.current_usable_bytes, value.live_blocks), (0, 0));
        assert!(!value.diagnostic_error);
    }

    #[test]
    fn windows_include_preexisting_allocations_and_conservative_realloc_overlap() {
        let allocator = Census::new(System);
        let old_layout = Layout::from_size_align(128, 8).unwrap();
        let pointer = unsafe { allocator.alloc(old_layout) };
        assert!(!pointer.is_null());
        let before = allocator.snapshot().current_usable_bytes;
        let window = allocator.begin_window().unwrap();
        assert!(matches!(
            allocator.begin_window(),
            Err(WindowError::AlreadyActive)
        ));
        assert_eq!(
            window.snapshot().unwrap().baseline_current_usable_bytes,
            before
        );
        let grown = unsafe { allocator.realloc(pointer, old_layout, 4096) };
        assert!(!grown.is_null());
        let after = allocator.snapshot().current_usable_bytes;
        let measured = window.snapshot().unwrap();
        assert!(measured.peak_usable_bytes >= before + after);
        unsafe {
            allocator.dealloc(grown, Layout::from_size_align(4096, 8).unwrap());
        }
        let end = window.finish().unwrap();
        assert_eq!(end.current_usable_bytes, 0);
        assert!(!end.diagnostic_error);
        let next = allocator.begin_window().unwrap();
        assert_eq!(next.snapshot().unwrap().baseline_current_usable_bytes, 0);
        assert_ne!(next.snapshot().unwrap().generation, end.generation);
        next.finish().unwrap();
    }

    #[test]
    fn concurrent_allocations_and_snapshots_are_serialized() {
        let allocator = Census::new(System);
        let window = allocator.begin_window().unwrap();
        let filled = Barrier::new(5);
        let release = Barrier::new(5);
        let (observation, live_blocks) = std::thread::scope(|scope| {
            for _ in 0..4 {
                let allocator = &allocator;
                let filled = &filled;
                let release = &release;
                scope.spawn(move || {
                    let layout = Layout::from_size_align(2048, 64).unwrap();
                    let pointer = unsafe { allocator.alloc(layout) };
                    if !pointer.is_null() {
                        unsafe {
                            pointer.write_bytes(1, 2048);
                        }
                    }
                    filled.wait();
                    release.wait();
                    if !pointer.is_null() {
                        unsafe {
                            allocator.dealloc(pointer, layout);
                        }
                    }
                });
            }
            filled.wait();
            let observation = window.snapshot();
            let live_blocks = allocator.snapshot().live_blocks;
            release.wait();
            (observation, live_blocks)
        });
        let observation = observation.unwrap();
        assert!(observation.current_usable_bytes >= 8192);
        assert_eq!(live_blocks, 4);
        let value = window.finish().unwrap();
        assert_eq!(value.current_usable_bytes, 0);
        assert!(value.peak_usable_bytes >= 8192);
        assert!(!value.diagnostic_error);
    }

    #[test]
    fn dropped_window_latches_failure_and_releases_only_its_generation() {
        let allocator = Census::new(System);
        drop(allocator.begin_window().unwrap());
        assert!(allocator.snapshot().diagnostic_error);
        let next = allocator.begin_window().unwrap();
        assert!(next.finish().unwrap().diagnostic_error);
    }

    #[test]
    fn real_global_allocator_observes_owned_rust_buffers() {
        let window = GLOBAL.begin_window().unwrap();
        let mut bytes = vec![0_u8; 65536];
        std::hint::black_box(&mut bytes);
        let value = window.snapshot().unwrap();
        assert!(value.current_usable_bytes >= 65536);
        assert!(value.peak_usable_bytes >= value.current_usable_bytes);
        assert!(!value.diagnostic_error);
        drop(bytes);
        let end = window.finish().unwrap();
        assert!(end.peak_usable_bytes >= 65536);
        assert!(!end.diagnostic_error);
    }
}
