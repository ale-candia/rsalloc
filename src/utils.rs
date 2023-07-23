pub fn is_power_of_two(x: usize) -> bool {
    (x & (x - 1)) == 0
}

pub fn align_forward(mut addr: usize, alignment: usize) -> usize {
    assert!(is_power_of_two(alignment));

    // Same as (addr % alignment) but faster as 'alignment' is a power of two
    let modulo = addr & (alignment - 1);

    if modulo != 0 {
        addr += alignment - modulo;
    }

    addr
}

pub fn calc_padding_with_header(ptr: usize, alignment: usize, header_size: usize) -> usize {
    assert!(is_power_of_two(alignment));

    // (ptr % alignment) as 'alignment' is a power of two
    let modulo = ptr & (alignment - 1);

    let mut padding = 0;

    if modulo != 0 {
        padding = alignment - modulo;
    }

    // the header is smaller than the padding we're done
    // else align the left over (header - padding)
    if padding < header_size {
        let diff = header_size - padding;

        if (diff & (alignment - 1)) != 0 {
            padding += alignment * (1 + diff / alignment);
        } else {
            padding = header_size;
        }
    }

    padding
}

pub fn ref_as_usize<T>(var_ref: &T) -> usize {
    var_ref as *const T as usize
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_align_forward() {
        assert_eq!(align_forward(10, 4), 12);
        assert_eq!(align_forward(20, 8), 24);
        assert_eq!(align_forward(100, 32), 128);
    }

    #[test]
    #[should_panic]
    fn test_align_forward_with_non_power_of_two() {
        align_forward(10, 5);
    }

    #[test]
    fn test_calc_padding_with_header() {
        assert_eq!(calc_padding_with_header(3, 8, 8), 13);
        assert_eq!(calc_padding_with_header(3, 8, 29), 29);
    }
}
