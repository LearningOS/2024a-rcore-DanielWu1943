//! Process management syscalls
use core::intrinsics::size_of;
use core::num;

use alloc::vec::Vec;
use riscv::paging::PageTableEntry;
use crate::mm::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum};
use crate::mm::{frame_alloc, FrameTracker};
use crate::mm::{PTEFlags, PageTable, PageTableEntry};
use crate::task::current_user_token;
use crate::{
    config::{MAX_SYSCALL_NUM, PAGE_SIZE},
    task::{
        change_program_brk, exit_current_and_run_next, suspend_current_and_run_next, TaskStatus, TASK_MANAGER,
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
    let mut time_val_ptr = &time_val as *const _ as *const u8;

    //2. virt -> phys and check
    let buffers = translated_byte_buffer(current_user_token(), _ts as *const u8, size_of::<TimeVal>());//利用translated_byte_buffer将虚拟地址转换到物理地址。输入：页表的token，表示当前app的虚拟页表，虚拟地址的起始地址，虚拟地址长度
    for buffer in buffers{
        unsafe {
            time_val_ptr.copy_to(buffer.as_mut_ptr(), buffer.len());//buffer.as_mut_ptr()返回buffer的原始指针，buffer.len是buffer的长度，即将time_val_ptr复制到buffer.as_mut_ptr()为起始地址的地方，长度为buffer.len
            time_val_ptr = time_val_ptr.add(buffer.len());//返回一个新的指针，即time_val_ptr的值要等于原本time_val_ptr的值加上buffer len，即移动指针到下一个内存位置
        }
    }
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!("kernel: sys_task_info");
    // 获取当前任务
    let task = TASK_MANAGER.get_current_task();
    
    // 获取系统调用相关信息
    let syscall_times = task.syscall_times;
    
    // 获取任务状态
    let task_status = task.task_status;
    let time_sys_call = get_time_us();//系统调用的时间（当前时间）

    // 计算从任务第一次调度到当前的时间差（单位：毫秒）
    //由于start_time是Option类型，先解包
    if let Some(start_time) = task.start_time {
        let elapsed_time_ms = (time_sys_call - start_time) / 1000; // 计算时间差（单位：毫秒）
        
        // 构建 TaskInfo
        let task_info = TaskInfo {
            status: task_status,           // 任务状态
            syscall_times,                 // 系统调用次数
            time: elapsed_time_ms,         // 从任务第一次调度到当前的时间差
        };
        let buffers = translated_byte_buffer(current_user_token(), _ti as *const u8, size_of::<TaskInfo>());
        let mut task_info_ptr = &task_info as *const _ as *const u8;

        // 遍历缓冲区，将 TaskInfo 数据写入
        for buffer in buffers {
            unsafe {
                task_info_ptr.copy_to(buffer.as_mut_ptr(), buffer.len());
                task_info_ptr = task_info_ptr.add(buffer.len());
            }
        }
    } else {
        // 如果 start_time 为 None，任务尚未开始，进行相应处理
        return -1; // 根据实际需求决定返回值，可以选择返回错误码
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
    /* 
    // 1. 申请物理内存
    let num_page = (_len + PAGE_SIZE - 1) / PAGE_SIZE;
    let mut physical_memory = Vec::new();
    for _ in 0..num_page{
        match frame_alloc() {
        Some(frame) => physical_memory.push(frame), // 因为frame_alloc是分配一页物理内存，所以我们得先计算_len需要多少页物理内存，然后依次分配到phys_memory中。
        None => return -1, // 物理内存分配失败
        }
    }
    // 2. 获取虚拟地址范围
    let start_va = VirtAddr::from(_start);   // 将起始虚拟地址转换为 VirtAddr 类型
    let end_va = VirtAddr::from(_start + _len); // 计算结束虚拟地址

    // 3. 设置权限
    let permission = MapPermission::from(_port);  // 假设 port 可以转换为对应的权限

    // 4. 插入映射
    let page_table = current_user_token();  // 假设这是获取当前进程的页表

    let mut current_start_va = start_va;
    for frame in physical_memory {
        // 映射每一页物理内存到虚拟地址空间
        let next_start_va = current_start_va.add(PAGE_SIZE);  // 获取下一页的虚拟地址
        
        // 映射每一页物理内存到虚拟地址空间
        page_table.insert_framed_area(current_start_va, next_start_va, permission);
        
        current_start_va = next_start_va;  // 更新虚拟地址，映射下一页
    }*/
    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap!");
    /* 
    let mut memory_set = current_user_token();
    // 1. 遍历 areas 找到对应的 map_area
    let mut found = false;
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);

    // 从 areas 中找出所有与给定地址范围重叠的区域
    for i in 0..memory_set.areas.len() {
        let map_area = &mut memory_set.areas[i];

        // 如果当前映射区域与待取消的区域重叠
        if map_area.start_va >= start_va && map_area.end_va <= end_va {
            found = true;
            
            // 2. 从 areas 中移除对应的 map_area
            memory_set.areas.remove(i);

            // 3. 调用 unmap 取消映射
            map_area.unmap(&mut memory_set.page_table);
            
            break; // 找到并取消映射后可以退出循环
        }
    }

    if found {
        0 // 成功取消映射
    } else {
        -1 // 未找到对应的映射区域
    }*/
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
