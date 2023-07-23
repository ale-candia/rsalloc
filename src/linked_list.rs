use super::utils::{calc_padding_with_header, ref_as_usize};
use super::{Arena, SpinLock};
use core::alloc::{GlobalAlloc, Layout};
use core::mem::size_of;
use core::ptr;

pub enum PlacementPolicy {
    FindFirst,
    FindBest,
}

struct AllocationHeader {
    block_size: usize,
    padding: usize,
}

pub struct FreeListAllocator<'a> {
    arena: Arena,

    head: Option<&'a FreeNode<'a>>,
    policy: PlacementPolicy,

    initialized: bool,
}

struct FreeNode<'a> {
    next: Option<&'a FreeNode<'a>>,
    block_size: usize,
}

impl FreeListAllocator<'_> {
    pub const fn new(policy: PlacementPolicy) -> Self {
        Self {
            arena: Arena::new(),
            head: None,
            policy,
            initialized: false,
        }
    }

    fn init(&mut self) {
        self.initialized = true;

        // write the free node into the arena
        let node_addr = self.arena.start() as *mut FreeNode;
        let node = FreeNode {
            block_size: self.arena.size(),
            next: None,
        };
        unsafe { ptr::write(node_addr, node) };

        // make the head of the allocator point to the free node
        self.head = Some(unsafe { &*node_addr });
    }
}

// iterates over the entire list and finds the best fit
fn find_best<'a>(
    node: &'a FreeNode<'a>,
    size: usize,
    align: usize,
) -> (Option<&'a FreeNode<'a>>, Option<&'a FreeNode<'a>>, usize) {
    let mut node = Some(node);
    let mut prev_node: Option<&FreeNode> = None;

    let mut prev_to_best: Option<&FreeNode> = None;
    let mut best_node: Option<&FreeNode> = None;

    let mut padding: usize = 0;

    let mut smallest_diff = usize::MAX;

    while let Some(val) = node {
        let node_addr = ref_as_usize(val);
        padding = calc_padding_with_header(node_addr, align, size_of::<FreeNode>());

        let required_space = size + padding;

        if val.block_size >= required_space && (val.block_size - required_space < smallest_diff) {
            prev_to_best = prev_node;
            best_node = Some(val);
            smallest_diff = val.block_size - required_space;
        }

        prev_node = node;
        node = val.next;
    }

    // prev_node is always the last node
    (best_node, prev_to_best, padding)
}

// iterates the list and finds the first free block with enough space
fn find_first<'a>(
    node: &'a FreeNode<'a>,
    size: usize,
    align: usize,
) -> (Option<&'a FreeNode<'a>>, Option<&'a FreeNode<'a>>, usize) {
    let mut node = Some(node);
    let mut prev_node: Option<&FreeNode> = None;
    let mut first_node: Option<&FreeNode> = None;

    let mut padding: usize = 0;

    while let Some(val) = node {
        let node_addr = ref_as_usize(val);
        padding = calc_padding_with_header(node_addr, align, size_of::<FreeNode>());

        let required_space = size + padding;

        if val.block_size >= required_space {
            first_node = Some(val);
            break;
        }

        prev_node = node;
        node = val.next;
    }

    (first_node, prev_node, padding)
}

unsafe impl GlobalAlloc for SpinLock<FreeListAllocator<'_>> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let guard = self.lock();

        let mut allocator = guard.get_mut();

        if !allocator.initialized {
            allocator.init();
        }

        // allocator out of memory
        if allocator.head.is_none() {
            SpinLock::unlock(guard);
            return ptr::null_mut();
        }

        let size = if layout.size() < size_of::<FreeNode>() {
            size_of::<FreeNode>()
        } else {
            layout.size()
        };

        let alignment = if layout.align() < 8 {
            8
        } else {
            layout.align()
        };

        // if we reach this section then the head is not null, i.e. there is a at least one free node
        // free_node will still be none if the data doesn't fit
        let (free_node, prev_node, padding) = match allocator.policy {
            PlacementPolicy::FindFirst => find_first(allocator.head.unwrap(), size, alignment),
            PlacementPolicy::FindBest => find_best(allocator.head.unwrap(), size, alignment),
        };

        // not enough memory left
        if free_node.is_none() {
            SpinLock::unlock(guard);
            return ptr::null_mut();
        }
        let free_node_addr = ref_as_usize(free_node.unwrap());

        // remove the selected node from the list
        if let Some(prev_node_ref) = prev_node {
            // if there is a previous node then update it to point to the next FreeNode
            let prev_node_addr = prev_node_ref as *const FreeNode as *mut FreeNode;
            let new_prev_node = FreeNode {
                block_size: prev_node_ref.block_size,
                next: free_node.unwrap().next,
            };
            unsafe { ptr::write(prev_node_addr, new_prev_node) };
        } else {
            // if the previous node is None, this means the head is the next free area
            if let Some(val) = free_node.unwrap().next {
                allocator.head = Some(val);
            } else {
                // if there is no next area, resize the free area if possible
                let remaining = free_node.unwrap().block_size as i128 - (padding + size) as i128;

                if remaining > 0 {
                    let new_free_node = FreeNode {
                        block_size: remaining.try_into().unwrap(),
                        next: None,
                    };
                    let new_free_node_addr = free_node_addr.checked_add(padding + size).unwrap();
                    unsafe { ptr::write(new_free_node_addr as *mut FreeNode, new_free_node) };

                    allocator.head = unsafe { Some(&*(new_free_node_addr as *const FreeNode)) };
                } else {
                    allocator.head = None;
                }
            }
        }

        // insert the header into the memory region
        if free_node.unwrap().block_size > padding + size {
            let header = AllocationHeader {
                block_size: padding + size,
                padding,
            };
            let header_addr = free_node_addr + padding - size_of::<AllocationHeader>();

            unsafe { ptr::write(header_addr as *mut AllocationHeader, header) };
        }

        let ptr = (free_node_addr + padding) as *mut u8;
        SpinLock::unlock(guard);

        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        let guard = self.lock();
        let mut allocator = guard.get_mut();
        let ptr_addr = ptr as usize;

        // allocation header corresponding to this allocation
        let alloc_header = unsafe {
            let alloc_header_addr = ptr_addr - size_of::<AllocationHeader>();
            ptr::read(alloc_header_addr as *const AllocationHeader)
        };

        // create a new free node
        let mut free_node = FreeNode {
            block_size: alloc_header.block_size,
            next: None,
        };
        let mut free_node_addr = ptr_addr - alloc_header.padding;

        // if there is no free nodes, make this free node the head
        if allocator.head.is_none() {
            let free_node_ref = unsafe {
                ptr::write(free_node_addr as *mut FreeNode, free_node);

                &*(free_node_addr as *const FreeNode)
            };

            allocator.head = Some(free_node_ref);
            SpinLock::unlock(guard);
            return;
        }

        // if there are free nodes, insert the created node into the list keeping it sorted
        let mut node = allocator.head;
        let mut prev_node: Option<&FreeNode> = None;

        let mut update_prev = true;

        while let Some(val) = node {
            if ref_as_usize(val) > free_node_addr {
                free_node.next = Some(val);

                // coalesce to the previous region if possible
                if let Some(prev_val) = prev_node {
                    if ref_as_usize(prev_val) + prev_val.block_size == free_node_addr {
                        update_prev = false;

                        free_node.block_size += val.block_size;
                        free_node_addr = ref_as_usize(prev_val);
                    }
                }

                // coalesce to the next region if possible
                if free_node_addr + free_node.block_size == ref_as_usize(val) {
                    free_node.block_size += val.block_size;
                    free_node.next = val.next;
                }

                // if there is no node before this one, then make this the head of the list
                if prev_node.is_none() {
                    allocator.head = unsafe { Some(&*(free_node_addr as *const FreeNode)) };
                }

                break;
            }

            prev_node = node;
            node = val.next;
        }

        // update the previous node to point to this new deallocated free space
        if update_prev && prev_node.is_some() {
            let prev_node_value = FreeNode {
                block_size: prev_node.unwrap().block_size,
                next: unsafe { Some(&*(free_node_addr as *const FreeNode)) },
            };
            unsafe {
                ptr::write(
                    prev_node.unwrap() as *const FreeNode as *mut FreeNode,
                    prev_node_value,
                )
            };
        }

        unsafe { ptr::write(free_node_addr as *mut FreeNode, free_node) };

        SpinLock::unlock(guard);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_find_first() {
        let node_3 = FreeNode {
            block_size: 100,
            next: None,
        };
        let node_2 = FreeNode {
            block_size: 50,
            next: Some(&node_3),
        };
        let node_1 = FreeNode {
            block_size: 75,
            next: Some(&node_2),
        };
        let head = FreeNode {
            block_size: 10,
            next: Some(&node_1),
        };

        let (free_node, prev_node, _) = find_first(&head, 20, 2);

        assert_eq!(free_node.unwrap().block_size, node_1.block_size);
        assert_eq!(prev_node.unwrap().block_size, head.block_size);
    }

    #[test]
    fn test_find_best() {
        let node_3 = FreeNode {
            block_size: 100,
            next: None,
        };
        let node_2 = FreeNode {
            block_size: 50,
            next: Some(&node_3),
        };
        let node_1 = FreeNode {
            block_size: 75,
            next: Some(&node_2),
        };
        let head = FreeNode {
            block_size: 10,
            next: Some(&node_1),
        };

        let (free_node, prev_node, _) = find_best(&head, 20, 2);

        assert_eq!(free_node.unwrap().block_size, node_2.block_size);

        assert_eq!(prev_node.unwrap().block_size, node_1.block_size);
    }

    #[test]
    fn test_allocation_deallocation_find_first() {
        let global_alloc_first: SpinLock<FreeListAllocator> =
            SpinLock::new(FreeListAllocator::new(PlacementPolicy::FindFirst));

        let layout_u32 = Layout::new::<u32>();
        let layout_u64 = Layout::new::<[u64; 34]>();

        let ptr_1 = unsafe { global_alloc_first.alloc(layout_u32) };
        assert!(!ptr_1.is_null());

        let ptr_2 = unsafe { global_alloc_first.alloc(layout_u64) };
        assert!(!ptr_2.is_null());

        // a pointer to a new location was given
        assert!(ptr_1 < ptr_2);

        unsafe { global_alloc_first.dealloc(ptr_1, layout_u32) };

        let ptr_3 = unsafe { global_alloc_first.alloc(layout_u32) };
        assert!(!ptr_3.is_null());

        // the first free area is used
        assert_eq!(ptr_1 as usize, ptr_3 as usize);

        unsafe { global_alloc_first.dealloc(ptr_3, layout_u32) };
        unsafe { global_alloc_first.dealloc(ptr_2, layout_u64) };
    }

    //
    //   We fragment the memory this way and find the best fit.
    //
    //   +----+--------+----+----+----+------------+
    //   |xxxx|        |xxxx|    |xxxx|            |
    //   |xxxx|        |xxxx|    |xxxx|            |
    //   |xxxx|        |xxxx|    |xxxx|            |
    //   +----+--------+----+----+----+------------+
    //
    #[test]
    fn test_allocation_deallocation_find_best() {
        let global_alloc_best: SpinLock<FreeListAllocator> =
            SpinLock::new(FreeListAllocator::new(PlacementPolicy::FindBest));

        let layout_u32 = Layout::new::<u32>();
        let layout_u64 = Layout::new::<[u64; 34]>();

        unsafe { global_alloc_best.alloc(layout_u32) };

        let large_section_1 = unsafe { global_alloc_best.alloc(layout_u64) };
        assert!(!large_section_1.is_null());

        unsafe { global_alloc_best.alloc(layout_u32) };

        let best_fit_section = unsafe { global_alloc_best.alloc(layout_u32) };
        assert!(!best_fit_section.is_null());

        unsafe { global_alloc_best.alloc(layout_u32) };

        unsafe { global_alloc_best.dealloc(large_section_1, layout_u64) };
        unsafe { global_alloc_best.dealloc(best_fit_section, layout_u32) };

        let ptr = unsafe { global_alloc_best.alloc(layout_u32) };
        assert_eq!(ptr as usize, best_fit_section as usize);
    }
}
