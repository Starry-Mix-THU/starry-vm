use core::alloc::Layout;

use axerrno::{LinuxError, LinuxResult};
use memory_addr::{MemoryAddr, PAGE_SIZE_4K, VirtAddr, VirtAddrRange};
use page_table_multiarch::MappingFlags;

use crate::AddrSpaceProvider;

#[percpu::def_percpu]
static mut ACCESSING_USER_MEM: bool = false;

/// Check if we are currently accessing user memory.
///
/// OS implementation shall allow page faults from kernel when this function
/// returns true.
pub fn is_accessing_user_memory() -> bool {
    ACCESSING_USER_MEM.read_current()
}

fn access_user_memory<R>(f: impl FnOnce() -> R) -> R {
    ACCESSING_USER_MEM.with_current(|v| {
        *v = true;
        let result = f();
        *v = false;
        result
    })
}

/// A wrapper structure for user address space pointers.
///
/// It provides a series of convenient methods for checking
/// and accessing user-mode areas.
///
/// It requires the caller to provide some methods for accessing
/// the address space, which is encapsulated in the [`AddrSpaceProvider`]
/// trait.
#[repr(transparent)]
pub struct UserPtr<A: AddrSpaceProvider, T> {
    data: *mut T,
    _marker: core::marker::PhantomData<A>,
}

impl<A: AddrSpaceProvider, T> From<usize> for UserPtr<A, T> {
    fn from(value: usize) -> Self {
        UserPtr {
            data: value as *mut T,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<A: AddrSpaceProvider, T: Eq + Default> UserPtr<A, T> {
    /// Check whether the access operation to a certain region is legal.
    ///
    /// If this region is not accessible or the operation doesn't have enough
    /// permissions, this function will return an error.
    ///
    /// # Arguments
    ///
    /// - `start`: The start address of the region
    /// - `layout`: The layout of the area, including size and alignment of this region
    /// - `access_flags`: The access flags of this operation
    pub fn check_region(
        start: VirtAddr,
        layout: Layout,
        access_flags: MappingFlags,
    ) -> LinuxResult<()> {
        let align = layout.align();
        if start.as_usize() & (align - 1) != 0 {
            return Err(LinuxError::EFAULT);
        }

        if !A::check_region_access(
            VirtAddrRange::from_start_size(start, layout.size()),
            access_flags,
        ) {
            return Err(LinuxError::EFAULT);
        }

        let page_start = start.align_down_4k();
        let page_end = (start + layout.size()).align_up_4k();
        A::populate_area(page_start, page_end - page_start)?;

        Ok(())
    }

    /// Check whether a given continuous non-empty area is legal
    ///
    /// This function starts from the given area location and checks
    /// whether the entire area is accessible until the next empty
    /// character position.
    ///
    /// If the check passes, it returns the starting point of the accessible
    /// area and the length of the corresponding non-empty area.
    ///
    /// # Arguments
    ///
    /// - `start`: The start address of the region
    /// - `access_flags`: The access flags of this operation
    pub fn check_null_terminated(
        start: VirtAddr,
        access_flags: MappingFlags,
    ) -> LinuxResult<(*const T, usize)> {
        let align = Layout::new::<T>().align();
        if start.as_usize() & (align - 1) != 0 {
            return Err(LinuxError::EFAULT);
        }

        let zero = T::default();

        let mut page = start.align_down_4k();

        let start = start.as_ptr_of::<T>();
        let mut len = 0;

        access_user_memory(|| {
            loop {
                // SAFETY: This won't overflow the address space since we'll check
                // it below.
                let ptr = unsafe { start.add(len) };
                while ptr as usize >= page.as_ptr() as usize {
                    // We cannot prepare `aspace` outside of the loop, since holding
                    // aspace requires a mutex which would be required on page
                    // fault, and page faults can trigger inside the loop.

                    // TODO: this is inefficient, but we have to do this instead of
                    // querying the page table since the page might has not been
                    // allocated yet.
                    if !A::check_region_access(
                        VirtAddrRange::from_start_size(page, PAGE_SIZE_4K),
                        access_flags,
                    ) {
                        return Err(LinuxError::EFAULT);
                    }

                    page += PAGE_SIZE_4K;
                }

                // This might trigger a page fault
                // SAFETY: The pointer is valid and points to a valid memory region.
                if unsafe { ptr.read_volatile() } == zero {
                    break;
                }
                len += 1;
            }
            Ok(())
        })?;

        Ok((start, len))
    }
}

impl<A: AddrSpaceProvider, T> UserPtr<A, T> {
    pub const ACCESS_FLAGS: MappingFlags = MappingFlags::READ.union(MappingFlags::WRITE);

    /// Get the address of the pointer.
    pub fn address(&self) -> VirtAddr {
        VirtAddr::from_mut_ptr_of(self.data)
    }

    /// Unwrap the pointer into a raw pointer.
    ///
    /// # Safety
    /// This function is unsafe because it assumes that the pointer is valid and
    /// points to a valid memory region.
    pub unsafe fn as_ptr(&self) -> *mut T {
        self.data
    }

    /// Cast the pointer to a different type.
    pub fn cast<U>(self) -> UserPtr<A, U> {
        UserPtr {
            data: self.data as *mut U,
            _marker: core::marker::PhantomData,
        }
    }

    /// Check if the pointer is null.
    pub fn is_null(&self) -> bool {
        self.data.is_null()
    }

    /// Convert the pointer into an `Option`.
    ///
    /// This function returns `None` if the pointer is null, and `Some(self)`
    /// otherwise.
    pub fn nullable(self) -> Option<Self> {
        if self.is_null() { None } else { Some(self) }
    }
}
