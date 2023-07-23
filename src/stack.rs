use super::utils::calc_padding_with_header;
use super::{Arena, SpinLock};
use core::alloc::{GlobalAlloc, Layout};
use core::mem::size_of;
use core::ptr;

// Not needed anymore since we're using usize for padding instead of u8
// which makes the `MAX_ALIGNMENT` huge
// max_alignment = 2 ^ (8 * sizeof(padding) − 1)
// const MAX_ALIGNMENT: usize = 128;

pub struct StackAllocator {
    arena: Arena,
    prev_offset: usize,
    curr_offset: usize,
}

impl StackAllocator {
    pub const fn new() -> Self {
        StackAllocator {
            arena: Arena::new(),
            prev_offset: 0,
            curr_offset: 0,
        }
    }
}

unsafe impl GlobalAlloc for SpinLock<StackAllocator> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Start of the critical section
        let guard = self.lock();

        let mut allocator = guard.get_mut();

        let curr_addr = allocator.curr_offset + allocator.arena.start();

        let padding_with_header =
            calc_padding_with_header(curr_addr, layout.align(), size_of::<StackHeader>());

        let end = curr_addr + padding_with_header + layout.size();

        if end > allocator.arena.end() {
            // stack allocator is out of memory
            SpinLock::unlock(guard);
            return ptr::null_mut();
        }

        // store the header
        let header_addr = curr_addr + padding_with_header - size_of::<StackHeader>();
        let header = StackHeader {
            prev_offset: allocator.prev_offset,
            padding: padding_with_header,
        };
        unsafe { ptr::write(header_addr as *mut StackHeader, header) };

        // update the offsets
        allocator.prev_offset = allocator.curr_offset;
        allocator.curr_offset = end - allocator.arena.start();

        SpinLock::unlock(guard);

        (curr_addr + padding_with_header) as *mut u8
    }

    // layout is unused, since we are not zeroing the memory, all the data will be left there but
    // overwritten whenever a new allocation occurs
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        let guard = self.lock();

        let mut allocator = guard.get_mut();

        let ptr_addr = ptr as usize;

        // memory out of bounds
        if !(allocator.arena.start() <= ptr_addr && ptr_addr < allocator.arena.end()) {
            return;
        }

        // the memory was not allocated yet
        if ptr_addr >= allocator.arena.start() + allocator.curr_offset {
            return;
        }

        let header_addr = (ptr_addr - size_of::<StackHeader>()) as *const StackHeader;
        let header = unsafe { ptr::read(header_addr) };

        let prev_offset = ptr_addr - header.padding - allocator.arena.start();

        // out of order stack allocator free, should do something to indicate that
        if prev_offset != allocator.prev_offset {
            return;
        }

        // reset offsets
        allocator.curr_offset = allocator.prev_offset;
        allocator.prev_offset = header.prev_offset;

        SpinLock::unlock(guard);
    }
}

struct StackHeader {
    prev_offset: usize,
    padding: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    static GLOBAL_ALLOC: SpinLock<StackAllocator> = SpinLock::new(StackAllocator::new());

    const HEADER_SIZE: usize = core::mem::size_of::<StackHeader>();

    #[test]
    fn test_allocation_deallocation() {
        let layout_u32 = Layout::new::<u32>();
        let layout_u64 = Layout::new::<[u64; 34]>();

        let ptr_1 = unsafe { GLOBAL_ALLOC.alloc(layout_u32) };
        // successful allocation and alignment
        assert!(!ptr_1.is_null());
        assert!(
            (ptr_1 as usize - HEADER_SIZE) % layout_u32.align() == 0
                || ptr_1 as usize % layout_u32.align() == 0
        );

        let ptr_2 = unsafe { GLOBAL_ALLOC.alloc(layout_u64) };
        // successful allocation and alignment
        assert!(!ptr_2.is_null());
        assert!(
            (ptr_2 as usize - HEADER_SIZE) % layout_u64.align() == 0
                || ptr_2 as usize % layout_u64.align() == 0
        );

        // a pointer to a new location was given
        assert!(ptr_1 < ptr_2);

        // deallocation test
        unsafe { GLOBAL_ALLOC.dealloc(ptr_2, layout_u64) };

        {
            let guard = GLOBAL_ALLOC.lock();
            let allocator = guard.get();

            let offset = ptr_1 as usize - allocator.arena.start() + layout_u32.size();

            assert_eq!(allocator.curr_offset, offset);
            SpinLock::unlock(guard);
        }

        unsafe { GLOBAL_ALLOC.dealloc(ptr_1, layout_u32) };

        {
            let guard = GLOBAL_ALLOC.lock();
            let allocator = guard.get();

            assert_eq!(allocator.curr_offset, 0);
            SpinLock::unlock(guard);
        }
    }
}
