//! Process management syscalls
//use core::intrinsics::size_of;
use crate::mm::{translated_byte_buffer, VirtAddr, MapPermission, VPNRange};
use crate::task::{current_user_token, get_current_status};
use crate::{
    config::{PAGE_SIZE, MAX_SYSCALL_NUM},
    task::{
        change_program_brk, exit_current_and_run_next, suspend_current_and_run_next, TaskStatus, get_current_task_page_table, unmap_consecutive_area, create_new_map_area, get_syscall_times, get_start_time,  
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
/// sys_get_time是系统调用，但是_ts是来自用户空间的指针（即是虚拟地址），首先需要确保虚拟地址指向的内存是可访问的，即先将它映射到物理地址。
/// 利用current_user_token()获取当前app的页表，然后调用translated_byte_buffer将_ts映射到物理地址。
/// 
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    //1. get time
    let us = get_time_us();
    let time_val = TimeVal{
        sec: us / 1000000,
        usec: us % 1000000,
    };
    let time_val_ptr = &time_val as *const _ as *const u8;

    //2. virt -> phys and check
    let dst_vec = translated_byte_buffer(current_user_token(), _ts as *const u8, core::mem::size_of::<TimeVal>());//利用translated_byte_buffer将虚拟地址转换到物理地址。输入：页表的token，表示当前app的虚拟页表，虚拟地址的起始地址，虚拟地址长度
    
    
    for (idx, dst) in dst_vec.into_iter().enumerate(){
        let unit_len = dst.len();
        unsafe {
            dst.copy_from_slice(core::slice::from_raw_parts(time_val_ptr.wrapping_byte_add(idx * unit_len) as *const u8, unit_len));
        }
    }
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!("kernel: sys_task_info");
    // 获取当前任务，由于ch4的TCB没有clone所以没法直接获得current task，需要修改
    let status = get_current_status();
    let syscall_times = get_syscall_times();
    let start_time = get_start_time();
    // 计算从任务第一次调度到当前的时间差（单位：毫秒）
    //用引用绑定
    let ref task_info = TaskInfo {
        status,           // 任务状态
        syscall_times,                 // 系统调用次数
        time: start_time,         // 从任务第一次调度到当前的时间差
    };
    //2. virt -> phys and check
    let dst_vec = translated_byte_buffer(current_user_token(), _ti as *const u8, core::mem::size_of::<TaskInfo>());//利用translated_byte_buffer将虚拟地址转换到物理地址。输入：页表的token，表示当前app的虚拟页表，虚拟地址的起始地址，虚拟地址长度
    
    let task_info_ptr = task_info as *const TaskInfo;
    for (idx, dst) in dst_vec.into_iter().enumerate(){
        let unit_len = dst.len();
        unsafe {
            dst.copy_from_slice(core::slice::from_raw_parts(task_info_ptr.wrapping_byte_add(idx * unit_len) as *const u8, unit_len));//wrapping_byte_add:
        }
    }
    0 // 返回0表示成功
}

// YOUR JOB: Implement mmap.
///1. 确保页参数start是对齐的
///2. port校验：第 0 位表示是否可读，第 1 位表示是否可写，第 2 位表示是否可执行。其他位无效且必须为 0
/// insert_framed_area 方法是通过将一段虚拟地址范围（start_va 到 end_va）映射到物理内存来管理内存区域
/// sys_mmap是申请一块物理内存并将其映射到指定的虚拟地址范围，首先得申请物理内存
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!("kernel: sys_mmap");
    //1. 校验
    if _start % PAGE_SIZE != 0 ||
    _port & !0x07 != 0 ||
    _port & 0x07 == 0 {
        return -1;
    }

    //2. check [_start, _start + _len)是否有映射
    let start_vpn = VirtAddr::from(_start).floor();
    let end_vpn = VirtAddr::from(_start + _len).ceil();
    let vpns = VPNRange::new(start_vpn, end_vpn);//创建一个从start_vpn到end_vpn的range structure for virtual page number
    for vpn in vpns {
        //查看vpn对应的pte是否有数据
        if let Some(pte) = get_current_task_page_table(vpn) {
            // we find a pte that has been mapped
            if pte.is_valid() {
                return -1;
            }
       }
    }
    //3. 划分新区域
    create_new_map_area(start_vpn.into(), end_vpn.into(), MapPermission::from_bits_truncate((_port << 1) as u8) | MapPermission::U);
    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap!");
    if _start % PAGE_SIZE != 0 {
        return -1;
    }
    let mut map_len = _len;
    if _start > usize::MAX - _len {
        map_len = usize::MAX - _start;
    }
    unmap_consecutive_area(_start, map_len);
    0
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
