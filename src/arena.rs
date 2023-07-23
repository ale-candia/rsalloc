use super::ARENA_SIZE;
use core::cell::UnsafeCell;

pub struct Arena {
    arena: UnsafeCell<[u8; ARENA_SIZE]>,
}

impl Arena {
    pub const fn new() -> Self {
        Self {
            arena: UnsafeCell::new([0x00; ARENA_SIZE]),
        }
    }

    #[inline]
    pub fn start(&self) -> usize {
        self.arena.get() as usize
    }

    #[inline]
    pub fn end(&self) -> usize {
        self.start() + ARENA_SIZE
    }

    #[inline(always)]
    pub fn size(&self) -> usize {
        ARENA_SIZE
    }
}
