use super::{Arena, SpinLock, ARENA_SIZE};
use core::alloc::GlobalAlloc;
use core::ptr;

pub struct PoolAllocator<'a> {
    arena: Arena,
    chunk_size: usize,
    head: Option<&'a PoolFreeNode<'a>>,
    initialized: bool,
}

struct PoolFreeNode<'a> {
    next: Option<&'a PoolFreeNode<'a>>,
}

#[allow(dead_code)]
impl PoolAllocator<'_> {
    pub const fn new(chunk_size: usize) -> Self {
        Self {
            arena: Arena::new(),
            chunk_size,
            head: None,
            initialized: false,
        }
    }

    fn init(&mut self) {
        self.initialized = true;

        let chunk_count: usize = ARENA_SIZE / self.chunk_size;

        let mut prev_node: Option<&PoolFreeNode> = None;

        for i in 0..chunk_count {
            let offset = i * self.chunk_size;

            // allocate the node in chunk `i`
            let node = PoolFreeNode { next: None };

            // save the current header onto the arena
            let node_pointer = (self.arena.start() + offset) as *mut PoolFreeNode;
            let node_reference = unsafe {
                ptr::write(node_pointer, node);

                // get a reference to the written node
                &*node_pointer as &PoolFreeNode
            };

            // if there is a previous node, make the node point to the new node
            if let Some(val) = prev_node {
                let node = PoolFreeNode {
                    next: Some(node_reference),
                };

                // write the new value
                unsafe {
                    ptr::write(val as *const PoolFreeNode as *mut PoolFreeNode, node);
                }
            }

            // make the new node the previous node
            prev_node = Some(node_reference);
        }

        // the head is the first allocated node
        self.head = unsafe { Some(&*(self.arena.start() as *const PoolFreeNode)) };
    }
}

unsafe impl GlobalAlloc for SpinLock<PoolAllocator<'_>> {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let guard = self.lock();

        let mut allocator = guard.get_mut();

        if layout.size() > allocator.chunk_size {
            panic!("data doesn't fit in chunk");
        }

        if !allocator.initialized {
            allocator.init();
        }

        if let Some(head) = allocator.head {
            let ptr_addr = head as *const PoolFreeNode as usize;

            allocator.head = head.next;

            SpinLock::unlock(guard);
            ptr_addr as *mut u8
        } else {
            SpinLock::unlock(guard);
            ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        let guard = self.lock();

        let mut allocator = guard.get_mut();

        // ignore deallocation if not initialized
        if !allocator.initialized {
            return;
        }
        // memory out of bounds
        if !(allocator.arena.start() <= (ptr as usize) && (ptr as usize) < allocator.arena.end()) {
            return;
        }

        let node = PoolFreeNode {
            next: allocator.head,
        };
        let node_pointer = ptr as *mut PoolFreeNode;

        // write the node to the arena and get a reference to it
        let node_reference = unsafe {
            ptr::write(node_pointer, node);
            &*node_pointer as &PoolFreeNode
        };

        allocator.head = Some(node_reference);

        SpinLock::unlock(guard);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    static GLOBAL_ALLOC: SpinLock<PoolAllocator> = SpinLock::new(PoolAllocator::new(1024));

    #[test]
    fn test_init() {
        let mut pool = PoolAllocator::new(1024);

        pool.init();

        assert_eq!(pool.initialized, true);

        let mut chunk_count = 0;
        while let Some(head) = pool.head {
            pool.head = head.next;
            chunk_count += 1;
        }

        // size of the allocator is ARENA_SIZE = 128 * 1024
        // so we expect 128 chunks
        assert_eq!(chunk_count, 128);
    }

    #[test]
    fn test_allocation_deallocation() {
        let layout_u32 = Layout::new::<u32>();
        let layout_u64 = Layout::new::<[u64; 34]>();

        let ptr_1 = unsafe { GLOBAL_ALLOC.alloc(layout_u32) };
        assert!(!ptr_1.is_null());

        let ptr_2 = unsafe { GLOBAL_ALLOC.alloc(layout_u64) };
        assert!(!ptr_2.is_null());

        // a pointer to a new location was given
        assert!(ptr_1 < ptr_2);

        // deallocation tests
        unsafe { GLOBAL_ALLOC.dealloc(ptr_1, layout_u32) };
        unsafe { GLOBAL_ALLOC.dealloc(ptr_2, layout_u64) };

        let guard = GLOBAL_ALLOC.lock();

        let head = guard.get().head.unwrap();
        assert_eq!(ptr_2 as usize, head as *const PoolFreeNode as usize);

        let head_next = head.next.unwrap();
        assert_eq!(ptr_1 as usize, head_next as *const PoolFreeNode as usize);

        SpinLock::unlock(guard);
    }
}
