#![no_std]
#![feature(const_generics)]
#![allow(incomplete_features)]

use crate::PageHeader::{Erased, Active, GcRunning};

#[repr(usize)]
enum PageHeader {
    Erased = core::usize::MAX,
    Active = 1,
    GcRunning = 0,
}

pub trait EEPROM<const N: usize> {
    unsafe fn get_pages(&self) -> [&[usize]; N];
    unsafe fn get_pages_mut(&mut self) -> [&mut [usize]; N];
    unsafe fn reset_page(&mut self, index: usize);

    fn run_garbage_collection(&mut self) -> usize {
        let mut pages = unsafe { self.get_pages_mut() };

        let mut maybe_active_page_index = None;

        for (index, page) in pages.iter().enumerate() {
            match page[0] {
                core::usize::MAX => continue,
                1 => {
                    maybe_active_page_index = Some(index);
                    break;
                }
                _ => panic!("run_garbage_collection: invalid page header {}", page[0])
            }
        };

        let active_page_index = if let Some(n) = maybe_active_page_index {
            n
        } else {
            for i in 0..pages.len() {
                unsafe { self.reset_page(i); }
            }

            pages = unsafe { self.get_pages_mut() };
            pages[0][0] = 1;
            0
        };

        let next_page_index = if active_page_index + 1 == pages.len() {
            0
        } else {
            active_page_index + 1
        };

        let (active_page, next_page) = get_two_mut(&mut pages, active_page_index, next_page_index);

        assert_eq!(next_page[0], Erased as usize);

        active_page[0] = GcRunning as usize;

        let mut active_index = 1;
        let mut next_index = 1;

        // Copy the variables from the active page into the next page
        loop {
            let address = active_page[active_index];
            let length = active_page[active_index + 1];

            match address {
                core::usize::MAX => break,
                0 => active_index = active_index + 2 + length,
                _ => {
                    let data = &active_page[active_index + 2..active_index + 2 + length];

                    next_page[next_index] = address;
                    next_page[next_index + 1] = length;
                    next_page[next_index + 2..next_index + 2 + length].copy_from_slice(&data);

                    next_index = next_index + 2 + length;
                }
            }
        }


        // Reset the active page
        unsafe { self.reset_page(active_page_index) };

        let pages = unsafe { self.get_pages_mut() };

        // Set the next page to the active state
        pages[next_page_index][0] = Active as usize;

        next_page_index
    }


    fn write(&mut self, address: usize, data: &[usize]) {
        assert_ne!(address, 0);
        assert_ne!(address, core::usize::MAX);

        let mut pages = unsafe { self.get_pages_mut() };

        let mut maybe_active_page_index = None;

        for (index, page) in pages.iter().enumerate() {
            match page[0] {
                core::usize::MAX => continue,
                1 => {
                    maybe_active_page_index = Some(index);
                    break;
                }
                _ => panic!("write: invalid page header {}", page[0])
            }
        };

        let active_page_index = if let Some(n) = maybe_active_page_index {
            n
        } else {
            for i in 0..pages.len() {
                unsafe { self.reset_page(i); }
            }

            pages = unsafe { self.get_pages_mut() };
            pages[0][0] = 1;
            0
        };

        let mut page = &mut pages[active_page_index];

        let mut index = 1;
        let mut gc_run = false;

        loop {
            let length = page[index + 1];

            if index + 2 + data.len() > page.len() {
                if gc_run {
                    panic!("Not enough space in eeprom to write to address {}", address);
                } else {
                    let page_index = self.run_garbage_collection();
                    pages = unsafe { self.get_pages_mut() };
                    page = &mut pages[page_index];
                    index = 1;
                    gc_run = true;
                }
            }

            if page[index] == core::usize::MAX {
                page[index] = address;
                page[index + 1] = data.len();
                page[index + 2..index + 2 + data.len()].copy_from_slice(data);
                return;
            } else if page[index] == address {
                page[index] = 0;
                index = index + 2 + length;
            } else {
                index = index + 2 + length;
            }
        }
    }

    fn read(&self, address: usize) -> Option<&[usize]> {
        assert_ne!(address, 0);
        assert_ne!(address, core::usize::MAX);

        let pages = unsafe { self.get_pages() };

        let mut maybe_active_page_index = None;

        for (index, page) in pages.iter().enumerate() {
            match page[0] {
                core::usize::MAX => continue,
                1 => {
                    maybe_active_page_index = Some(index);
                    break;
                }
                _ => panic!("read: invalid page header {}", page[0])
            }
        };

        let active_page_index: usize = if let Some(n) = maybe_active_page_index {
            n
        } else {
            return None;
        };

        let page = pages[active_page_index];

        let mut index = 1;

        loop {
            let length = page[index + 1];

            if index >= page.len() {
                return None;
            }

            if page[index] == core::usize::MAX {
                return None;
            } else if page[index] == address {
                return Some(&page[index + 2..index + 2 + length]);
            } else {
                index = index + 2 + length;
            }
        }
    }
}

/// Returns a mutable reference to two elements of a slice
///
/// # Panics
///
/// Panics if `a` or `b` are out of bounds.
/// Panics if `a` and `b` are equal.
fn get_two_mut<T>(slice: &mut [T], a: usize, b: usize) -> (&mut T, &mut T) {
    assert_ne!(a, b);

    unsafe {
        let ar = &mut *(slice.get_mut(a).unwrap() as *mut _);
        let br = &mut *(slice.get_mut(b).unwrap() as *mut _);
        (ar, br)
    }
}

#[cfg(test)]
mod tests {
    use crate::EEPROM;

    struct ArrayEEPROM {
        pages: [[usize; 1024]; 3]
    }

    impl ArrayEEPROM {
        fn new() -> ArrayEEPROM {
            ArrayEEPROM { pages: [[core::usize::MAX; 1024]; 3] }
        }
    }

    impl EEPROM<3> for ArrayEEPROM {
        unsafe fn get_pages(&self) -> [&[usize]; 3] {
            [&self.pages[0], &self.pages[1], &self.pages[2]]
        }

        unsafe fn get_pages_mut(&mut self) -> [&mut [usize]; 3] {
            [
                &mut *(self.pages.get_unchecked_mut(0) as *mut _),
                &mut *(self.pages.get_unchecked_mut(1) as *mut _),
                &mut *(self.pages.get_unchecked_mut(2) as *mut _)
            ]
        }

        unsafe fn reset_page(&mut self, index: usize) {
            self.pages[index] = [core::usize::MAX; 1024];
        }
    }

    #[test]
    fn write_and_read() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();
        let data = [1, 2, 3, 4];

        eeprom.write(1, &data);
        assert_eq!(eeprom.read(1).unwrap(), &data)
    }

    #[test]
    fn read_missing_returns_none() {
        let eeprom: ArrayEEPROM = ArrayEEPROM::new();

        assert_eq!(eeprom.read(1), None)
    }

    #[test]
    #[should_panic]
    fn write_too_much() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();
        let data = [1; 1035];

        eeprom.write(1, &data);
    }

    #[test]
    fn run_the_garbage_collector() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();

        for i in 0..16 {
            eeprom.write(1, &[i; 512]);
            for j in eeprom.read(1).unwrap() {
                assert_eq!(j, &i);
            }
        }
    }

    #[test]
    #[should_panic]
    fn write_address_zero_panics() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.write(0, &[1]);
    }

    #[test]
    #[should_panic]
    fn write_address_max_panics() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.write(core::usize::MAX, &[1]);
    }

    #[test]
    #[should_panic]
    fn read_address_zero_panics() {
        let eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.read(0);
    }

    #[test]
    #[should_panic]
    fn read_address_max_panics() {
        let eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.read(core::usize::MAX);
    }
}
