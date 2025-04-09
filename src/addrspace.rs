use axerrno::LinuxResult;
use memory_addr::{VirtAddr, VirtAddrRange};
use page_table_multiarch::MappingFlags;

pub trait AddrSpaceProvider {
    fn check_region_access(range: VirtAddrRange, access_flags: MappingFlags) -> bool;

    fn populate_area(start: VirtAddr, size: usize) -> LinuxResult<()>;
}
