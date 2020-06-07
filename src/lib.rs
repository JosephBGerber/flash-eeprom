#![no_std]
#![feature(const_generics)]
#![allow(incomplete_features)]

use byteorder::{ByteOrder, LittleEndian};

use crate::PageHeader::{Erased, Active, GcRunning};

#[repr(u8)]
enum PageHeader {
    Erased = core::u8::MAX,
    Active = 1,
    GcRunning = 0,
}

struct Variable {
    address: u8,
    size: u32,
}

impl Into<[u8; 5]> for Variable {
    fn into(self) -> [u8; 5] {
        let address = self.address;
        let size = self.size.to_le_bytes();

        [address, size[0], size[1], size[2], size[3]]
    }
}

impl Into<Variable> for &[u8] {
    fn into(self) -> Variable {
        assert_eq!(self.len(), 5);

        Variable {
            address: self[0],
            size: LittleEndian::read_u32(&self[1..5]),
        }
    }
}

pub trait EEPROM<const N: usize> {
    unsafe fn get_pages(&self) -> [&[u8]; N];
    unsafe fn get_pages_mut(&mut self) -> [&mut [u8]; N];
    unsafe fn reset_page(&mut self, index: usize);

    fn run_garbage_collection(&mut self) -> usize {
        let mut pages = unsafe { self.get_pages_mut() };

        let mut maybe_active_page_index = None;

        for (index, page) in pages.iter().enumerate() {
            match page[0] {
                core::u8::MAX => continue,
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

        assert_eq!(next_page[0], Erased as u8);

        active_page[0] = GcRunning as u8;

        let mut active_index = 1;
        let mut next_index = 1;

        // Copy the variables from the active page into the next page
        loop {
            let variable: Variable = active_page[active_index..active_index + 5].into();

            match variable.address {
                core::u8::MAX => break,
                0 => active_index = active_index + 5 + variable.size as usize,
                _ => {
                    let data = &active_page[active_index + 5..active_index + 5 + variable.size as usize];

                    next_page[next_index..next_index + 5].copy_from_slice(&active_page[active_index..active_index + 5]);
                    next_page[next_index + 5..next_index + 5 + variable.size as usize].copy_from_slice(&data);

                    next_index = next_index + 5 + variable.size as usize;
                }
            }
        }


        // Reset the active page
        unsafe { self.reset_page(active_page_index) };

        let pages = unsafe { self.get_pages_mut() };

        // Set the next page to the active state
        pages[next_page_index][0] = Active as u8;

        next_page_index
    }


    fn write_variable(&mut self, address: u8, data: &[u8]) {
        assert_ne!(address, 0);
        assert_ne!(address, core::u8::MAX);

        let mut pages = unsafe { self.get_pages_mut() };

        let mut maybe_active_page_index = None;

        for (index, page) in pages.iter().enumerate() {
            match page[0] {
                core::u8::MAX => continue,
                1 => {
                    maybe_active_page_index = Some(index);
                    break;
                }
                _ => panic!("write_variable: invalid page header {}", page[0])
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
            let variable: Variable = page[index..index + 5].into();

            if index + 5 + data.len() > page.len() {
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

            if variable.address == core::u8::MAX {
                page[index] = address;
                page[index + 1..index + 5].copy_from_slice(&(data.len() as u32).to_le_bytes());
                page[index + 5..index + 5 + data.len()].copy_from_slice(data);
                return;
            } else if page[index] == address {
                page[index] = 0;
                index = index + 5 + variable.size as usize;
            } else {
                index = index + 5 + variable.size as usize;
            }
        }
    }

    fn read_variable(&self, address: u8) -> Option<&[u8]> {
        assert_ne!(address, 0);
        assert_ne!(address, core::u8::MAX);

        let pages = unsafe { self.get_pages() };

        let mut maybe_active_page_index = None;

        for (index, page) in pages.iter().enumerate() {
            match page[0] {
                core::u8::MAX => continue,
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
            let variable: Variable = page[index..index + 5].into();

            if index >= page.len() {
                return None;
            }

            if variable.address == core::u8::MAX {
                return None;
            } else if variable.address == address {
                return Some(&page[index + 5..index + 5 + variable.size as usize]);
            } else {
                index = index + 5 + variable.size as usize;
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
        pages: [[u8; 4096]; 3]
    }

    impl ArrayEEPROM {
        fn new() -> ArrayEEPROM {
            ArrayEEPROM { pages: [[core::u8::MAX; 4096]; 3] }
        }
    }

    impl EEPROM<3> for ArrayEEPROM {
        unsafe fn get_pages(&self) -> [&[u8]; 3] {
            [&self.pages[0], &self.pages[1], &self.pages[2]]
        }

        unsafe fn get_pages_mut(&mut self) -> [&mut [u8]; 3] {
            [
                &mut *(self.pages.get_unchecked_mut(0) as *mut _),
                &mut *(self.pages.get_unchecked_mut(1) as *mut _),
                &mut *(self.pages.get_unchecked_mut(2) as *mut _)
            ]
        }

        unsafe fn reset_page(&mut self, index: usize) {
            self.pages[index] = [core::u8::MAX; 4096];
        }
    }

    #[test]
    fn write_and_read() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();
        let data = [1, 2, 3, 4];

        eeprom.write_variable(1, &data);
        assert_eq!(eeprom.read_variable(1).unwrap(), &data)
    }

    #[test]
    fn read_missing_returns_none() {
        let eeprom: ArrayEEPROM = ArrayEEPROM::new();

        assert_eq!(eeprom.read_variable(1), None)
    }

    #[test]
    #[should_panic]
    fn write_too_much() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();
        let data = [1; 4097];

        eeprom.write_variable(1, &data);
    }

    #[test]
    fn run_the_garbage_collector() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();

        for i in 0..16 {
            eeprom.write_variable(1, &[i; 512]);
            for j in eeprom.read_variable(1).unwrap() {
                assert_eq!(j, &i);
            }
        }
    }

    #[test]
    #[should_panic]
    fn write_address_zero_panics() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.write_variable(0, &[1]);
    }

    #[test]
    #[should_panic]
    fn write_address_max_panics() {
        let mut eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.write_variable(core::u8::MAX, &[1]);
    }

    #[test]
    #[should_panic]
    fn read_address_zero_panics() {
        let eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.read_variable(0);
    }

    #[test]
    #[should_panic]
    fn read_address_max_panics() {
        let eeprom: ArrayEEPROM = ArrayEEPROM::new();

        eeprom.read_variable(core::u8::MAX);
    }
}
