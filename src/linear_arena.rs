use super::utils::align_forward;
use super::{Arena, SpinLock, ARENA_SIZE};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

pub struct ArenaAllocator {
    arena: Arena,
    curr_offset: usize,
}

impl ArenaAllocator {
    pub const fn new() -> Self {
        ArenaAllocator {
            arena: Arena::new(),
            curr_offset: 0,
        }
    }
}

unsafe impl GlobalAlloc for SpinLock<ArenaAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Start of the critical section
        let guard = self.lock();

        let allocator = guard.get_mut();

        // start position of the new allocation
        let start = align_forward(
            allocator.curr_offset + allocator.arena.start(),
            layout.align(),
        );

        // end position of the new allocation
        let end = match start.checked_add(layout.size()) {
            Some(end) => end,
            None => {
                SpinLock::unlock(guard);
                return ptr::null_mut();
            }
        };

        if end > start + ARENA_SIZE {
            // arena out of memory
            SpinLock::unlock(guard);
            return ptr::null_mut();
        }

        // update the offset
        allocator.curr_offset = end - allocator.arena.start();

        SpinLock::unlock(guard);

        start as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // arena allocator doesn't allow to free certain blocks of memory
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    static GLOBAL_ARENA: SpinLock<ArenaAllocator> = SpinLock::new(ArenaAllocator::new());

    #[test]
    fn single_alignment() {
        let layout = Layout::new::<u32>();

        let ptr_1 = unsafe { GLOBAL_ARENA.alloc(layout) };
        assert!(!ptr_1.is_null());
        assert_eq!(ptr_1 as usize % layout.align(), 0);

        let ptr_2 = unsafe { GLOBAL_ARENA.alloc(layout) };
        assert!(!ptr_2.is_null());
        assert_eq!(ptr_2 as usize % layout.align(), 0);

        // a pointer to a new location was given
        assert_ne!(ptr_1, ptr_2);

        // size of u32  => 4
        // align of u32 => 4
        assert!(ptr_1 as usize + 4 == ptr_2 as usize);
    }

    #[test]
    fn multiple_alignment() {
        let layout_u32 = Layout::new::<u32>();
        let layout_u64 = Layout::new::<u64>();

        let ptr_1 = unsafe { GLOBAL_ARENA.alloc(layout_u32) };
        assert!(!ptr_1.is_null());
        assert_eq!(ptr_1 as usize % layout_u32.align(), 0);

        let ptr_2 = unsafe { GLOBAL_ARENA.alloc(layout_u64) };
        assert!(!ptr_2.is_null());
        assert_eq!(ptr_2 as usize % layout_u64.align(), 0);

        // a pointer to a new location was given
        assert_ne!(ptr_1, ptr_2);

        // size of u32  => 4
        // align of u32 => 4
        // size of u64  => 8
        // align of u64 => 8
        assert!(ptr_1 as usize + 8 == ptr_2 as usize);
    }
}
